use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use subtle::ConstantTimeEq;
use tokio_util::sync::CancellationToken;

use super::Device;
use crate::pack::PackError;

const CODE_LIFETIME: Duration = Duration::from_secs(300);
const MINUTE: Duration = Duration::from_secs(60);
const MAX_PEERS: usize = 1024;
const TOKEN: u64 = 1_000_000_000;

pub(super) struct Session {
    pub device: Device,
    pub content: String,
    pub api: String,
    pub csrf: String,
    pub revoked: CancellationToken,
}

struct Peer {
    seen: Instant,
    refill: Instant,
    credit: u64,
    failures_since: Instant,
    failures: u8,
    locked_until: Option<Instant>,
}

pub(super) struct Sessions {
    room: crate::room::Room,
    pub stopped: CancellationToken,
    cap: usize,
    code: String,
    code_expires: Instant,
    sessions: Vec<Arc<Session>>,
    peers: HashMap<IpAddr, Peer>,
    global_since: Instant,
    global_failures: u8,
}

impl Sessions {
    pub fn new(cap: usize, now: Instant) -> Result<Self, PackError> {
        if !(1..=128).contains(&cap) {
            return Err(PackError::Mismatch("LAN device cap must be 1..128".into()));
        }
        Ok(Self {
            room: crate::room::Room::new()
                .map_err(|_| PackError::Io("LAN room initialization".into()))?,
            stopped: CancellationToken::new(),
            cap,
            code: random_code()?,
            code_expires: now + CODE_LIFETIME,
            sessions: Vec::new(),
            peers: HashMap::new(),
            global_since: now,
            global_failures: 0,
        })
    }

    pub fn code(&mut self, now: Instant) -> Result<String, PackError> {
        if self.stopped.is_cancelled() {
            return Err(PackError::Mismatch("LAN server is stopped".into()));
        }
        if now >= self.code_expires {
            self.code = random_code()?;
            self.code_expires = now + CODE_LIFETIME;
        }
        Ok(self.code.clone())
    }

    fn peer(&mut self, ip: IpAddr, now: Instant) -> Result<&mut Peer, StatusCode> {
        if !self.peers.contains_key(&ip) && self.peers.len() >= MAX_PEERS {
            self.peers.retain(|_, peer| {
                now.saturating_duration_since(peer.seen) < Duration::from_secs(600)
            });
            if self.peers.len() >= MAX_PEERS {
                return Err(StatusCode::TOO_MANY_REQUESTS);
            }
        }
        let peer = self.peers.entry(ip).or_insert(Peer {
            seen: now,
            refill: now,
            credit: 64 * TOKEN,
            failures_since: now,
            failures: 0,
            locked_until: None,
        });
        peer.seen = now;
        Ok(peer)
    }

    pub fn admit(&mut self, ip: IpAddr, now: Instant) -> Result<(), StatusCode> {
        let peer = self.peer(ip, now)?;
        let elapsed = now
            .saturating_duration_since(peer.refill)
            .as_nanos()
            .min(2_000_000_000) as u64;
        peer.credit = peer.credit.saturating_add(elapsed * 32).min(64 * TOKEN);
        peer.refill = now;
        if peer.credit < TOKEN {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        peer.credit -= TOKEN;
        Ok(())
    }

    pub fn pair(
        &mut self,
        ip: IpAddr,
        code: &str,
        label: &str,
        now: Instant,
    ) -> Result<Arc<Session>, StatusCode> {
        if self.stopped.is_cancelled() {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        if label.is_empty() || label.len() > 64 || label.chars().any(char::is_control) {
            return Err(StatusCode::BAD_REQUEST);
        }
        if now.saturating_duration_since(self.global_since) >= MINUTE {
            self.global_since = now;
            self.global_failures = 0;
        }
        let valid = now < self.code_expires && equal(code, &self.code);
        let globally_locked = self.global_failures >= 20;
        let peer = self.peer(ip, now)?;
        if peer.locked_until.is_some_and(|until| now < until) || globally_locked {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        if now.saturating_duration_since(peer.failures_since) >= MINUTE {
            peer.failures_since = now;
            peer.failures = 0;
        }
        if !valid {
            peer.failures += 1;
            let locked = peer.failures >= 5;
            if locked {
                peer.locked_until = Some(now + CODE_LIFETIME);
            }
            self.global_failures += 1;
            return Err(if locked || self.global_failures >= 20 {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::UNAUTHORIZED
            });
        }
        if self.sessions.len() >= self.cap {
            return Err(StatusCode::CONFLICT);
        }
        let session = Arc::new(Session {
            device: Device {
                id: random_token().map_err(unavailable)?,
                label: label.into(),
                peer: ip,
            },
            content: random_token().map_err(unavailable)?,
            api: random_token().map_err(unavailable)?,
            csrf: random_token().map_err(unavailable)?,
            revoked: self.stopped.child_token(),
        });
        self.sessions.push(Arc::clone(&session));
        Ok(session)
    }

    pub fn authenticate(&self, token: &str, api: bool) -> Option<Arc<Session>> {
        self.sessions
            .iter()
            .find(|session| {
                !session.revoked.is_cancelled()
                    && equal(token, if api { &session.api } else { &session.content })
            })
            .cloned()
    }

    pub fn devices(&self) -> Vec<Device> {
        self.sessions
            .iter()
            .map(|session| session.device.clone())
            .collect()
    }

    pub fn kick(&mut self, id: &str) -> bool {
        let Some(position) = self
            .sessions
            .iter()
            .position(|session| session.device.id == id)
        else {
            return false;
        };
        self.sessions.remove(position).revoked.cancel();
        self.room.remove_actor(id);
        true
    }

    pub fn stop(&mut self) {
        self.stopped.cancel();
        self.room.end();
        for session in self.sessions.drain(..) {
            session.revoked.cancel();
        }
    }
    pub fn room_host(
        &mut self,
        request: crate::room::RoomRequest,
    ) -> Result<crate::room::RoomSnapshot, crate::room::RoomError> {
        self.room.request(
            crate::room::RoomMember {
                id: "host".into(),
                label: "Host".into(),
                is_host: true,
            },
            request,
            Instant::now(),
        )
    }
    pub fn room_peer(
        &mut self,
        session: &Session,
        request: crate::room::RoomRequest,
    ) -> Result<crate::room::RoomSnapshot, crate::room::RoomError> {
        if session.revoked.is_cancelled()
            || !self
                .sessions
                .iter()
                .any(|active| active.device.id == session.device.id)
        {
            return Err(crate::room::RoomError::Unavailable);
        }
        self.room.request(
            crate::room::RoomMember {
                id: session.device.id.clone(),
                label: session.device.label.clone(),
                is_host: false,
            },
            request,
            Instant::now(),
        )
    }
}

pub(super) fn equal(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}
fn unavailable(_: PackError) -> StatusCode {
    StatusCode::SERVICE_UNAVAILABLE
}

fn random_token() -> Result<String, PackError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| PackError::Io(error.to_string()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn random_code() -> Result<String, PackError> {
    loop {
        let mut bytes = [0_u8; 4];
        getrandom::fill(&mut bytes).map_err(|error| PackError::Io(error.to_string()))?;
        let value = u32::from_le_bytes(bytes);
        if value < u32::MAX - u32::MAX % 1_000_000 {
            return Ok(format!("{:06}", value % 1_000_000));
        }
    }
}
