//! AP-only DNS for OS captive-network probes. Never forwards or caches queries.
use std::{io, net::Ipv4Addr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::watch,
    task::JoinSet,
};
use wifi_setupd_zbus_api::{BackendState, LinkState, ProvisioningState, Status};

pub(crate) fn active(status: Status) -> bool {
    status.backend == BackendState::Available
        && status.state == ProvisioningState::AwaitingCredentials
        && status.link != LinkState::Ready
}

/// Answer one uncompressed IN question. Ignore malformed/unsupported messages.
/// A records resolve to the AP; AAAA/HTTPS/other types receive empty answers so
/// IPv4 fallback works without inventing IPv6 connectivity or HTTPS endpoints.
fn answer(query: &[u8], ip: Ipv4Addr) -> Option<Vec<u8>> {
    if query.len() < 17
        || query[2] & 0xf8 != 0
        || query[4..6] != [0, 1]
        || query[6..10] != [0, 0, 0, 0]
    {
        return None;
    }
    let mut end = 12;
    loop {
        let len = usize::from(*query.get(end)?);
        end += 1;
        if len == 0 {
            break;
        }
        if len > 63 {
            return None;
        }
        end = end.checked_add(len)?;
        if end > query.len() || end - 12 > 254 {
            return None;
        }
    }
    let kind = u16::from_be_bytes(query.get(end..end + 2)?.try_into().ok()?);
    let class = u16::from_be_bytes(query.get(end + 2..end + 4)?.try_into().ok()?);
    let mut response = query.get(..end + 4)?.to_vec();
    response[2] = 0x84 | (query[2] & 1); // response, authoritative, original RD
    response[3] = if class == 1 { 0 } else { 5 }; // REFUSED outside IN
    response[6..12].fill(0); // no echoed OPT/additional records
    if class == 1 && kind == 1 {
        response[7] = 1;
        // Name pointer to the validated question, A/IN, TTL zero, four bytes.
        response.extend_from_slice(&[0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 0, 0, 4]);
        response.extend_from_slice(&ip.octets());
    }
    Some(response)
}

async fn tcp_query(
    mut stream: TcpStream,
    ip: Ipv4Addr,
    status: watch::Receiver<Status>,
) -> io::Result<()> {
    let size = usize::from(stream.read_u16().await?);
    if !(17..=4096).contains(&size) {
        return Ok(());
    }
    let mut query = vec![0; size];
    stream.read_exact(&mut query).await?;
    if active(*status.borrow())
        && let Some(response) = answer(&query, ip)
    {
        stream.write_u16(response.len() as u16).await?;
        stream.write_all(&response).await?;
    }
    Ok(())
}

pub(crate) async fn serve(
    udp: UdpSocket,
    tcp: TcpListener,
    ip: Ipv4Addr,
    mut status: watch::Receiver<Status>,
) -> io::Result<()> {
    let mut buffer = [0u8; 4096];
    let mut clients = JoinSet::new();
    loop {
        if !active(*status.borrow()) {
            return Ok(());
        }
        tokio::select! {
            changed = status.changed() => { if changed.is_err() { return Ok(()); } }
            packet = udp.recv_from(&mut buffer) => {
                let (length, peer) = packet?;
                if active(*status.borrow()) && let Some(response) = answer(&buffer[..length], ip) {
                    udp.send_to(&response, peer).await?;
                }
            }
            connection = tcp.accept(), if clients.len() < 16 => {
                let (stream, _) = connection?;
                let status = status.clone();
                clients.spawn(async move { let _ = tokio::time::timeout(Duration::from_secs(2), tcp_query(stream, ip, status)).await; });
            }
            _ = clients.join_next(), if !clients.is_empty() => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn query(kind: u8) -> Vec<u8> {
        let mut q = vec![0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        q.extend_from_slice(b"\x07captive\x05apple\x03com\0");
        q.extend_from_slice(&[0, kind, 0, 1]);
        q
    }
    #[test]
    fn dns_answers_ipv4_without_caching_or_inventing_ipv6() {
        let ip = Ipv4Addr::new(192, 168, 16, 1);
        let q = query(1);
        let r = answer(&q, ip).unwrap();
        assert_eq!(&r[..2], &q[..2]);
        assert_eq!(r[7], 1);
        assert_eq!(&r[q.len() + 6..q.len() + 10], &[0; 4]);
        assert_eq!(&r[r.len() - 4..], &ip.octets());
        for kind in [28, 65, 255] {
            let q = query(kind);
            let r = answer(&q, ip).unwrap();
            assert_eq!(r.len(), q.len());
            assert_eq!(r[7], 0);
        }
    }
    #[test]
    fn malformed_queries_do_not_panic_or_echo_untrusted_sections() {
        let ip = Ipv4Addr::LOCALHOST;
        let q = query(1);
        for length in 0..q.len() {
            assert!(answer(&q[..length], ip).is_none());
        }
        for offset in [2, 4, 6, 12] {
            let mut q = q.clone();
            q[offset] = 0xc0;
            assert!(answer(&q, ip).is_none());
        }
        let mut q = query(1);
        q[11] = 1;
        q.extend_from_slice(&[0; 20]);
        let r = answer(&q, ip).unwrap();
        assert_eq!(&r[10..12], &[0, 0]);
        assert_eq!(r.len(), query(1).len() + 16);
    }
    #[tokio::test]
    async fn udp_and_tcp_serve_only_while_ap_is_ready_for_onboarding() {
        let ip = Ipv4Addr::LOCALHOST;
        let udp = UdpSocket::bind((ip, 0)).await.unwrap();
        let ua = udp.local_addr().unwrap();
        let tcp = TcpListener::bind((ip, 0)).await.unwrap();
        let ta = tcp.local_addr().unwrap();
        let ap = Status {
            state: ProvisioningState::AwaitingCredentials,
            backend: BackendState::Available,
            link: LinkState::Disconnected,
            ..Default::default()
        };
        let (tx, rx) = watch::channel(ap);
        let task = tokio::spawn(serve(udp, tcp, ip, rx));
        let client = UdpSocket::bind((ip, 0)).await.unwrap();
        client.send_to(&query(1), ua).await.unwrap();
        let mut bytes = [0; 512];
        let (n, _) = tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&bytes[n - 4..n], &ip.octets());
        let mut client = TcpStream::connect(ta).await.unwrap();
        let q = query(1);
        client.write_u16(q.len() as u16).await.unwrap();
        client.write_all(&q).await.unwrap();
        let n = client.read_u16().await.unwrap();
        let mut r = vec![0; n as usize];
        client.read_exact(&mut r).await.unwrap();
        assert_eq!(&r[r.len() - 4..], &ip.octets());
        tx.send_replace(Status {
            state: ProvisioningState::Idle,
            link: LinkState::Ready,
            ..ap
        });
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(TcpStream::connect(ta).await.is_err());
        assert!(!active(Status {
            state: ProvisioningState::Connecting,
            ..ap
        }));
        assert!(!active(Status {
            backend: BackendState::Unavailable,
            ..ap
        }));
    }
}
