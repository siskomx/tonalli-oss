//! Ephemeral paired LAN room; no files, private app state or AI conversation.
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum RoomAction {
    Get,
    Join,
    Send,
    Leave,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct RoomRequest {
    pub action: RoomAction,
    pub view: String,
    pub epoch: String,
    #[serde(default)]
    pub after: u64,
    #[serde(default)]
    pub text: String,
}
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct RoomMember {
    pub id: String,
    pub label: String,
    pub is_host: bool,
}
#[derive(Clone, Debug, Serialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct RoomMessage {
    pub id: u64,
    pub sender: RoomMember,
    pub text: String,
    pub created_ms: u64,
}
#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct RoomSnapshot {
    pub active: bool,
    pub epoch: String,
    pub self_id: String,
    pub joined: bool,
    pub latest: u64,
    pub messages: Vec<RoomMessage>,
    pub members: Vec<RoomMember>,
}
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[cfg_attr(feature = "ffi", derive(uniffi::Error))]
pub enum RoomError {
    #[error("Invalid room request")]
    Invalid,
    #[error("Join the room first")]
    NotJoined,
    #[error("Room is full")]
    Full,
    #[error("Send more slowly")]
    RateLimited,
    #[error("Room changed")]
    Changed,
    #[error("Sharing is off or unavailable")]
    Unavailable,
}
const TOKEN: u64 = 1_000_000_000;
const MAX_SEQUENCE: u64 = 9_007_199_254_740_991;
const LEASE: Duration = Duration::from_secs(30);
struct Rate {
    credit: u64,
    capacity: u64,
    updated: Instant,
}
impl Rate {
    fn new(tokens: u64, now: Instant) -> Self {
        Self {
            credit: tokens * TOKEN,
            capacity: tokens * TOKEN,
            updated: now,
        }
    }
    fn refill(&mut self, now: Instant) {
        let elapsed = now
            .saturating_duration_since(self.updated)
            .as_nanos()
            .min(2_000_000_000) as u64;
        self.credit = self
            .credit
            .saturating_add(elapsed * (self.capacity / TOKEN / 2))
            .min(self.capacity);
        self.updated = now;
    }
}
struct Presence {
    member: RoomMember,
    views: HashMap<String, Instant>,
    seen: Instant,
    rate: Rate,
}
pub struct Room {
    active: bool,
    epoch: String,
    latest: u64,
    messages: VecDeque<RoomMessage>,
    members: HashMap<String, Presence>,
    rate: Rate,
}
impl Room {
    pub fn new() -> Result<Self, RoomError> {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|_| RoomError::Unavailable)?;
        Ok(Self {
            active: true,
            epoch: bytes.iter().map(|b| format!("{b:02x}")).collect(),
            latest: 0,
            messages: VecDeque::new(),
            members: HashMap::new(),
            rate: Rate::new(16, Instant::now()),
        })
    }
    pub fn request(
        &mut self,
        actor: RoomMember,
        request: RoomRequest,
        now: Instant,
    ) -> Result<RoomSnapshot, RoomError> {
        let hex = |text: &str, length| {
            text.len() == length
                && text
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        if actor.is_host != (actor.id == "host")
            || (!actor.is_host && !hex(&actor.id, 64))
            || actor.label.is_empty()
            || actor.label.len() > 64
            || actor.label.chars().any(char::is_control)
            || !hex(&request.view, 32)
            || request.epoch.len() > 64
            || request.epoch.chars().any(char::is_control)
            || request.after > MAX_SEQUENCE
        {
            return Err(RoomError::Invalid);
        }
        if request.text.len() > 4096
            || request.text.chars().count() > 1000
            || request
                .text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
            || (request.action != RoomAction::Send && !request.text.is_empty())
        {
            return Err(RoomError::Invalid);
        }
        let text = request.text.replace("\r\n", "\n").replace('\r', "\n");
        let text = text.trim();
        if request.action == RoomAction::Send && text.is_empty() {
            return Err(RoomError::Invalid);
        }
        if !self.active {
            return if matches!(request.action, RoomAction::Get | RoomAction::Leave) {
                Ok(RoomSnapshot::default())
            } else {
                Err(RoomError::Unavailable)
            };
        }
        if matches!(request.action, RoomAction::Join | RoomAction::Send)
            && request.epoch != self.epoch
        {
            return Err(RoomError::Changed);
        }
        self.members.retain(|_, presence| {
            presence
                .views
                .retain(|_, seen| now.saturating_duration_since(*seen) < LEASE);
            !presence.views.is_empty() || now.saturating_duration_since(presence.seen) < LEASE
        });
        if self
            .members
            .get(&actor.id)
            .is_some_and(|p| p.member.label != actor.label || p.member.is_host != actor.is_host)
        {
            return Err(RoomError::Invalid);
        }
        match request.action {
            RoomAction::Get => {
                if let Some(presence) = self.members.get_mut(&actor.id) {
                    if let Some(seen) = presence.views.get_mut(&request.view) {
                        *seen = now;
                        presence.seen = now;
                    }
                }
            }
            RoomAction::Join => {
                if !self.members.contains_key(&actor.id) && self.members.len() >= 129 {
                    return Err(RoomError::Full);
                }
                let presence = self
                    .members
                    .entry(actor.id.clone())
                    .or_insert_with(|| Presence {
                        member: actor.clone(),
                        views: HashMap::new(),
                        seen: now,
                        rate: Rate::new(4, now),
                    });
                if !presence.views.contains_key(&request.view) && presence.views.len() >= 4 {
                    return Err(RoomError::Full);
                }
                presence.views.insert(request.view.clone(), now);
                presence.seen = now;
            }
            RoomAction::Leave => {
                if request.epoch == self.epoch {
                    if let Some(presence) = self.members.get_mut(&actor.id) {
                        presence.views.remove(&request.view);
                        presence.seen = now;
                    }
                } else {
                    return Ok(RoomSnapshot {
                        active: true,
                        epoch: self.epoch.clone(),
                        self_id: actor.id,
                        ..RoomSnapshot::default()
                    });
                }
            }
            RoomAction::Send => {
                let presence = self
                    .members
                    .get_mut(&actor.id)
                    .filter(|p| p.views.contains_key(&request.view))
                    .ok_or(RoomError::NotJoined)?;
                presence.rate.refill(now);
                self.rate.refill(now);
                if presence.rate.credit < TOKEN || self.rate.credit < TOKEN {
                    return Err(RoomError::RateLimited);
                }
                if self.latest >= MAX_SEQUENCE {
                    return Err(RoomError::Unavailable);
                }
                self.latest += 1;
                presence.rate.credit -= TOKEN;
                self.rate.credit -= TOKEN;
                presence.views.insert(request.view.clone(), now);
                presence.seen = now;
                let created_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
                    .min(8_000_000_000_000_000) as u64;
                if self.messages.len() == 100 {
                    self.messages.pop_front();
                }
                self.messages.push_back(RoomMessage {
                    id: self.latest,
                    sender: presence.member.clone(),
                    text: text.into(),
                    created_ms,
                });
            }
        }
        let joined = self
            .members
            .get(&actor.id)
            .is_some_and(|p| p.views.contains_key(&request.view));
        let mut result = RoomSnapshot {
            active: true,
            epoch: self.epoch.clone(),
            self_id: actor.id,
            joined,
            ..RoomSnapshot::default()
        };
        if joined {
            let after = if request.epoch == self.epoch && request.after <= self.latest {
                request.after
            } else {
                0
            };
            result.latest = self.latest;
            result.messages = self
                .messages
                .iter()
                .filter(|message| message.id > after)
                .cloned()
                .collect();
            result.members = self
                .members
                .values()
                .filter(|p| !p.views.is_empty())
                .map(|p| p.member.clone())
                .collect();
            result.members.sort_by(|a, b| {
                b.is_host
                    .cmp(&a.is_host)
                    .then_with(|| a.label.cmp(&b.label))
                    .then_with(|| a.id.cmp(&b.id))
            });
        }
        Ok(result)
    }
    pub fn remove_actor(&mut self, id: &str) {
        self.members.remove(id);
    }
    pub fn end(&mut self) {
        self.active = false;
        self.messages.clear();
        self.members.clear();
        self.latest = 0;
    }
}
