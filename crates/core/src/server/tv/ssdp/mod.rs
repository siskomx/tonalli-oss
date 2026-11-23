//! Owned IPv4 SSDP discovery. Loopback checks never advertise onto the LAN.
mod protocol;
use protocol::{Advertisements, MAX_AGE};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    io,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    sync::Arc,
    time::Duration,
};
use tokio::{net::UdpSocket as AsyncSocket, task::JoinSet, time::Instant};
use tokio_util::sync::CancellationToken;
pub(super) struct Prepared {
    listener: UdpSocket,
    sender: UdpSocket,
    destination: Option<SocketAddr>,
    advertisements: Arc<Advertisements>,
}
pub(super) struct Discovery {
    listener: AsyncSocket,
    sender: Arc<AsyncSocket>,
    destination: Option<SocketAddr>,
    advertisements: Arc<Advertisements>,
}
impl Prepared {
    pub fn new(bind: Ipv4Addr, http: SocketAddr, uuid: &str) -> io::Result<Self> {
        let listener = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        if !bind.is_loopback() {
            listener.set_reuse_address(true)?;
        }
        let local = if bind.is_loopback() {
            SocketAddr::from((bind, 0))
        } else {
            SocketAddr::from((Ipv4Addr::UNSPECIFIED, 1900))
        };
        listener.bind(&local.into())?;
        let destination = if bind.is_loopback() {
            None
        } else {
            listener.join_multicast_v4(&Ipv4Addr::new(239, 255, 255, 250), &bind)?;
            Some(SocketAddr::from((Ipv4Addr::new(239, 255, 255, 250), 1900)))
        };
        let sender = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        sender.bind(&SocketAddr::from((bind, 0)).into())?;
        if destination.is_some() {
            sender.set_multicast_if_v4(&bind)?;
            sender.set_multicast_ttl_v4(1)?;
        }
        listener.set_nonblocking(true)?;
        sender.set_nonblocking(true)?;
        Ok(Self {
            listener: listener.into(),
            sender: sender.into(),
            destination,
            advertisements: Arc::new(Advertisements::new(uuid, http)),
        })
    }
    #[cfg(test)]
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
    pub fn into_async(self) -> io::Result<Discovery> {
        Ok(Discovery {
            listener: AsyncSocket::from_std(self.listener)?,
            sender: Arc::new(AsyncSocket::from_std(self.sender)?),
            destination: self.destination,
            advertisements: self.advertisements,
        })
    }
}
impl Discovery {
    async fn announce(&self, alive: bool) -> io::Result<()> {
        let Some(destination) = self.destination else {
            return Ok(());
        };
        tokio::time::timeout(Duration::from_millis(200), async {
            for value in self.advertisements.notifications(alive) {
                self.sender.send_to(value.as_bytes(), destination).await?;
            }
            Ok(())
        })
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "TV announcement timed out"))?
    }
    pub async fn run(self, stop: CancellationToken, allow: fn(IpAddr) -> bool) -> io::Result<()> {
        self.announce(true).await?;
        let mut renewal = tokio::time::interval_at(
            Instant::now() + Duration::from_secs(MAX_AGE / 2),
            Duration::from_secs(MAX_AGE / 2),
        );
        renewal.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut jobs = JoinSet::new();
        let mut buffer = [0; 2049];
        let mut window = Instant::now();
        let mut received = 0u8;
        loop {
            tokio::select! {biased;
                _=stop.cancelled()=>break,
                result=jobs.join_next(),if !jobs.is_empty()=>{if let Some(Err(error))=result{return Err(io::Error::other(error));}},
                _=renewal.tick()=>self.announce(true).await?,
                packet=self.listener.recv_from(&mut buffer)=>{
                    let (size,peer)=packet?;
                    if !allow(peer.ip()) || peer.port()==0{continue;}
                    if window.elapsed()>=Duration::from_secs(1){window=Instant::now();received=0;}
                    if received>=8 || jobs.len()>=16{continue;}received+=1;
                    let Some(search)=protocol::search(&buffer[..size])else{continue;};
                    let replies=self.advertisements.responses(&search.target);if replies.is_empty(){continue;}
                    let mut random=[0;8];getrandom::fill(&mut random).map_err(io::Error::other)?;
                    let delay=Duration::from_millis(1+u64::from_le_bytes(random)%(search.wait.as_millis() as u64));
                    let sender=Arc::clone(&self.sender);
                    jobs.spawn(async move{tokio::time::sleep(delay).await;for reply in replies{sender.send_to(reply.as_bytes(),peer).await?;}Ok::<_,io::Error>(())});
                }
            }
        }
        jobs.abort_all();
        while jobs.join_next().await.is_some() {}
        let _ = self.announce(false).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn owned_announcements_send_alive_then_byebye_without_lan_traffic() {
        let client = AsyncSocket::bind("127.0.0.1:0").await.unwrap();
        let mut prepared = Prepared::new(
            Ipv4Addr::LOCALHOST,
            "127.0.0.1:48901".parse().unwrap(),
            "b512ca81-64d4-4380-b64b-0cec32b12fd2",
        )
        .unwrap();
        prepared.destination = Some(client.local_addr().unwrap());
        let stop = CancellationToken::new();
        let owner = tokio::spawn(
            prepared
                .into_async()
                .unwrap()
                .run(stop.clone(), |ip| ip.is_loopback()),
        );
        let mut buffer = [0; 2048];
        for _ in 0..5 {
            let (size, _) =
                tokio::time::timeout(Duration::from_secs(1), client.recv_from(&mut buffer))
                    .await
                    .unwrap()
                    .unwrap();
            let text = std::str::from_utf8(&buffer[..size]).unwrap();
            assert!(text.contains("NTS: ssdp:alive\r\n"));
            assert!(text.contains("LOCATION: http://127.0.0.1:48901/description.xml\r\n"));
        }
        stop.cancel();
        owner.await.unwrap().unwrap();
        for _ in 0..5 {
            let (size, _) =
                tokio::time::timeout(Duration::from_secs(1), client.recv_from(&mut buffer))
                    .await
                    .unwrap()
                    .unwrap();
            assert!(std::str::from_utf8(&buffer[..size])
                .unwrap()
                .contains("NTS: ssdp:byebye\r\n"));
        }
    }
}
