//! Finding the ways in: local addresses, STUN (public address for hole
//! punching), UPnP port mapping. Every candidate found here is offered to the
//! browser; WebRTC's ICE tries them all and the fastest working one wins.

use std::net::{IpAddr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

/// The address this machine uses to reach the internet over IPv4 (no packet
/// is sent: connecting a UDP socket only selects the interface).
pub fn local_ipv4() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_unspecified()).then_some(ip)
}

fn is_global_v6(ip: &Ipv6Addr) -> bool {
    let s = ip.segments();
    // 2000::/3 is global unicast. Excludes link-local, ULA (fc00::/7), loopback.
    (s[0] & 0xe000) == 0x2000
}

/// A global IPv6 address on this machine, if the network has IPv6.
pub fn global_ipv6() -> Option<IpAddr> {
    let s = UdpSocket::bind("[::]:0").ok()?;
    s.connect("[2001:4860:4860::8888]:9").ok()?;
    match s.local_addr().ok()?.ip() {
        IpAddr::V6(v6) if is_global_v6(&v6) => Some(IpAddr::V6(v6)),
        _ => None,
    }
}

const MAGIC: u32 = 0x2112_A442;

/// Ask STUN servers for this socket's public address. Must run before anything
/// else reads from the socket. Returns the first answer.
pub fn stun_mapped(sock: &UdpSocket, servers: &[String], timeout: Duration) -> Option<SocketAddr> {
    let v6 = sock.local_addr().ok()?.is_ipv6();
    let mut tids = Vec::new();
    for s in servers {
        let Ok(addrs) = s.to_socket_addrs() else { continue };
        let Some(addr) = addrs.into_iter().find(|a| a.is_ipv6() == v6) else { continue };
        let tid: [u8; 12] = crate::pake::random_bytes();
        let mut msg = Vec::with_capacity(20);
        msg.extend_from_slice(&0x0001u16.to_be_bytes());
        msg.extend_from_slice(&0u16.to_be_bytes());
        msg.extend_from_slice(&MAGIC.to_be_bytes());
        msg.extend_from_slice(&tid);
        if sock.send_to(&msg, addr).is_ok() {
            tids.push(tid);
        }
    }
    if tids.is_empty() {
        return None;
    }
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 1500];
    let old = sock.read_timeout().ok().flatten();
    let result = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break None;
        }
        let _ = sock.set_read_timeout(Some(left));
        let Ok((n, _)) = sock.recv_from(&mut buf) else { break None };
        if let Some(a) = parse_binding_response(&buf[..n], &tids) {
            break Some(a);
        }
    };
    let _ = sock.set_read_timeout(old);
    result
}

pub fn parse_binding_response(d: &[u8], tids: &[[u8; 12]]) -> Option<SocketAddr> {
    if d.len() < 20 || u16::from_be_bytes([d[0], d[1]]) != 0x0101 {
        return None;
    }
    let tid: [u8; 12] = d[8..20].try_into().ok()?;
    if !tids.contains(&tid) {
        return None;
    }
    let end = (20 + u16::from_be_bytes([d[2], d[3]]) as usize).min(d.len());
    let mut pos = 20;
    while pos + 4 <= end {
        let ty = u16::from_be_bytes([d[pos], d[pos + 1]]);
        let len = u16::from_be_bytes([d[pos + 2], d[pos + 3]]) as usize;
        let v = d.get(pos + 4..pos + 4 + len)?;
        if (ty == 0x0020 || ty == 0x0001) && v.len() >= 8 {
            let xor = ty == 0x0020;
            let mut port = u16::from_be_bytes([v[2], v[3]]);
            if xor {
                port ^= (MAGIC >> 16) as u16;
            }
            let mut key = MAGIC.to_be_bytes().to_vec();
            key.extend_from_slice(&tid);
            let ip = match v[1] {
                1 => {
                    let mut b: [u8; 4] = v[4..8].try_into().ok()?;
                    if xor {
                        b.iter_mut().zip(&key).for_each(|(x, k)| *x ^= k);
                    }
                    IpAddr::from(b)
                }
                2 if v.len() >= 20 => {
                    let mut b: [u8; 16] = v[4..20].try_into().ok()?;
                    if xor {
                        b.iter_mut().zip(&key).for_each(|(x, k)| *x ^= k);
                    }
                    IpAddr::from(b)
                }
                _ => return None,
            };
            return Some(SocketAddr::new(ip, port));
        }
        pos += 4 + len + (4 - len % 4) % 4;
    }
    None
}

/// A port opened on the home router for one session; closed again on drop.
pub struct UpnpMapping {
    gateway: igd_next::Gateway,
    pub external: SocketAddr,
}

impl Drop for UpnpMapping {
    fn drop(&mut self) {
        let _ = self.gateway.remove_port(igd_next::PortMappingProtocol::UDP, self.external.port());
    }
}

/// Map the same external port as `local` (or any free one) to `local`.
/// Fails quietly when the router has no UPnP, is behind CGNAT, or double NAT.
pub fn upnp_map(local: SocketAddr) -> Option<UpnpMapping> {
    let mut opts = igd_next::SearchOptions::default();
    opts.timeout = Some(Duration::from_millis(1500));
    opts.single_search_timeout = Some(Duration::from_millis(1500));
    let gateway = igd_next::search_gateway(opts).ok()?;
    let ext_ip = gateway.get_external_ip().ok()?;
    // A private "external" address means another NAT above the router: useless.
    let public = match ext_ip {
        IpAddr::V4(v4) => !(v4.is_private() || v4.is_loopback() || v4.is_link_local() || v4.octets()[0] == 100 && (v4.octets()[1] & 0xc0) == 64),
        IpAddr::V6(_) => false,
    };
    if !public {
        return None;
    }
    // Leases are 1 hour; a session that runs longer keeps working as long as
    // the router keeps the connection's state, and we renew below in future.
    let lease = 3600;
    for port in [local.port(), 0] {
        let ext_port = if port == 0 { 20000 + (u16::from_le_bytes(crate::pake::random_bytes()) % 40000) } else { port };
        if gateway.add_port(igd_next::PortMappingProtocol::UDP, ext_port, local, lease, "Everywhere").is_ok() {
            return Some(UpnpMapping { gateway, external: SocketAddr::new(ext_ip, ext_port) });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_xor_mapped_ipv4() {
        let tid = [9u8; 12];
        // XOR-MAPPED-ADDRESS for 203.0.113.5:40000
        let port = 40000u16 ^ (MAGIC >> 16) as u16;
        let ip = [203u8, 0, 113, 5];
        let m = MAGIC.to_be_bytes();
        let mut d = vec![0x01, 0x01, 0x00, 0x0c];
        d.extend_from_slice(&m);
        d.extend_from_slice(&tid);
        d.extend_from_slice(&[0x00, 0x20, 0x00, 0x08, 0x00, 0x01]);
        d.extend_from_slice(&port.to_be_bytes());
        d.extend(ip.iter().zip(m).map(|(a, b)| a ^ b));
        assert_eq!(parse_binding_response(&d, &[tid]), Some("203.0.113.5:40000".parse().unwrap()));
        assert_eq!(parse_binding_response(&d, &[[1u8; 12]]), None, "unknown transaction");
    }

    #[test]
    fn global_v6_filter() {
        assert!(is_global_v6(&"2a01:e0a::1".parse().unwrap()));
        assert!(!is_global_v6(&"fe80::1".parse().unwrap()));
        assert!(!is_global_v6(&"fd00::1".parse().unwrap()));
        assert!(!is_global_v6(&"::1".parse().unwrap()));
    }
}
