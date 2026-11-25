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
    stun_query(sock, servers, timeout, 1).into_iter().next()
}

/// Ask every STUN server from the same socket and return all the answers.
/// Two different public ports for one socket means the router changes the
/// port for each destination (a "strict" NAT).
pub fn stun_all(sock: &UdpSocket, servers: &[String], timeout: Duration) -> Vec<SocketAddr> {
    stun_query(sock, servers, timeout, usize::MAX)
}

/// Sends a Binding request to each server and resends every 400 ms to those
/// that have not answered (UDP loses packets), until `want` answers arrived
/// or `timeout` passed.
fn stun_query(sock: &UdpSocket, servers: &[String], timeout: Duration, want: usize) -> Vec<SocketAddr> {
    let Ok(local) = sock.local_addr() else { return Vec::new() };
    let v6 = local.is_ipv6();
    // (server address, transaction id, answered)
    let mut asks: Vec<(SocketAddr, [u8; 12], bool)> = Vec::new();
    for s in servers {
        let Ok(addrs) = s.to_socket_addrs() else { continue };
        if let Some(addr) = addrs.into_iter().find(|a| a.is_ipv6() == v6) {
            asks.push((addr, crate::pake::random_bytes(), false));
        }
    }
    let mut out = Vec::new();
    if asks.is_empty() {
        return out;
    }
    let tids: Vec<[u8; 12]> = asks.iter().map(|a| a.1).collect();
    let send = |addr: SocketAddr, tid: &[u8; 12]| {
        let mut msg = Vec::with_capacity(20);
        msg.extend_from_slice(&0x0001u16.to_be_bytes());
        msg.extend_from_slice(&0u16.to_be_bytes());
        msg.extend_from_slice(&MAGIC.to_be_bytes());
        msg.extend_from_slice(tid);
        let _ = sock.send_to(&msg, addr);
    };
    let deadline = Instant::now() + timeout;
    let resend_every = Duration::from_millis(400);
    let mut next_send = Instant::now();
    let mut buf = [0u8; 1500];
    let old = sock.read_timeout().ok().flatten();
    while out.len() < want.min(asks.len()) {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        if now >= next_send {
            for (addr, tid, done) in &asks {
                if !done {
                    send(*addr, tid);
                }
            }
            next_send = now + resend_every;
        }
        let wait = next_send.min(deadline).saturating_duration_since(Instant::now()).max(Duration::from_millis(1));
        let _ = sock.set_read_timeout(Some(wait));
        let Ok((n, _)) = sock.recv_from(&mut buf) else { continue };
        let d = &buf[..n];
        if let Some(a) = parse_binding_response(d, &tids) {
            if let Some(ask) = asks.iter_mut().find(|x| !x.2 && d.get(8..20) == Some(&x.1[..])) {
                ask.2 = true;
                out.push(a);
            }
        }
    }
    let _ = sock.set_read_timeout(old);
    out
}

/// How a NAT picks public ports, judged from the ports one socket got from
/// different servers.
#[derive(Debug, PartialEq)]
pub enum PortPattern {
    /// Same port for every destination: plain hole punching works.
    Stable,
    /// A new port per destination, counting up by `step`.
    Sequential { step: u16, last: u16 },
    /// A new port per destination, with no visible order.
    Random,
}

pub fn port_pattern(ports: &[u16]) -> PortPattern {
    let mut p: Vec<u16> = ports.to_vec();
    p.sort_unstable();
    p.dedup();
    if p.len() < 2 {
        return PortPattern::Stable;
    }
    let diffs: Vec<u16> = p.windows(2).map(|w| w[1] - w[0]).collect();
    if diffs.iter().all(|&d| d <= 64) {
        PortPattern::Sequential { step: *diffs.iter().min().unwrap_or(&1), last: *p.last().unwrap_or(&0) }
    } else {
        PortPattern::Random
    }
}

/// The browser's public IPv4 addresses (its "srflx" candidates in the SDP
/// offer), each with every port seen.
pub fn remote_public_v4(sdp: &str) -> Vec<(IpAddr, Vec<u16>)> {
    let mut out: Vec<(IpAddr, Vec<u16>)> = Vec::new();
    for line in sdp.lines() {
        let Some(rest) = line.trim().strip_prefix("a=candidate:") else { continue };
        let f: Vec<&str> = rest.split_whitespace().collect();
        // foundation component transport priority address port typ <type>
        if f.len() < 8 || !f[2].eq_ignore_ascii_case("udp") || f[6] != "typ" || f[7] != "srflx" {
            continue;
        }
        let (Ok(ip), Ok(port)) = (f[4].parse::<IpAddr>(), f[5].parse::<u16>()) else { continue };
        if !ip.is_ipv4() {
            continue;
        }
        match out.iter_mut().find(|(i, _)| *i == ip) {
            Some((_, ports)) => {
                if !ports.contains(&port) {
                    ports.push(port)
                }
            }
            None => out.push((ip, vec![port])),
        }
    }
    out
}

/// Ports to probe on a browser whose NAT counts up: the next `n` it is
/// likely to hand out (its connection to this PC is opened just after its
/// STUN queries, so it usually gets one of the next few).
pub fn predicted_ports(pattern: &PortPattern, n: u16) -> Vec<u16> {
    match *pattern {
        PortPattern::Sequential { step, last } => (1..=n)
            .filter_map(|k| last.checked_add(step.max(1).checked_mul(k)?))
            .filter(|&p| p >= 1024)
            .collect(),
        _ => Vec::new(),
    }
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

/// One line about this PC's network, printed at start: whether other
/// networks can reach it directly, and why not.
pub fn home_report(servers: &[String]) -> String {
    let v6 = global_ipv6().map(|ip| format!("yes ({ip})")).unwrap_or_else(|| "no".into());
    let Some(ip) = local_ipv4() else { return format!("no IPv4 network; IPv6: {v6}") };
    let Ok(sock) = UdpSocket::bind(SocketAddr::new(ip, 0)) else { return format!("IPv6: {v6}") };
    let answers = stun_all(&sock, servers, Duration::from_millis(2500));
    let Some(first) = answers.first() else {
        return format!("could not reach a STUN server (UDP blocked?); IPv6: {v6}");
    };
    let ports: Vec<u16> = answers.iter().map(SocketAddr::port).collect();
    let kind = match port_pattern(&ports) {
        PortPattern::Stable if answers.len() > 1 => "easy (same public port for everyone)",
        PortPattern::Stable => "unknown (only one STUN server answered)",
        _ => "strict (new public port per destination): connections from strict networks such as phone hotspots may fail",
    };
    format!("public IPv4 {}, NAT {kind}; IPv6: {v6}", first.ip())
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
    fn reads_browser_public_ports() {
        let sdp = "v=0\r\na=candidate:1 1 udp 2122260223 abcd.local 50000 typ host generation 0\r\n\
a=candidate:2 1 udp 1686052607 198.51.100.7 40001 typ srflx raddr 0.0.0.0 rport 0\r\n\
a=candidate:3 1 udp 1686052607 198.51.100.7 40002 typ srflx raddr 0.0.0.0 rport 0\r\n\
a=candidate:4 1 udp 1686052607 198.51.100.7 40002 typ srflx raddr 0.0.0.0 rport 0\r\n\
a=candidate:5 1 udp 1686052607 2001:db8::5 40003 typ srflx raddr :: rport 0\r\n";
        let r = remote_public_v4(sdp);
        assert_eq!(r, vec![("198.51.100.7".parse().unwrap(), vec![40001, 40002])]);
    }

    #[test]
    fn port_patterns() {
        assert_eq!(port_pattern(&[5000]), PortPattern::Stable);
        assert_eq!(port_pattern(&[5000, 5000]), PortPattern::Stable);
        assert_eq!(port_pattern(&[40002, 40001, 40003]), PortPattern::Sequential { step: 1, last: 40003 });
        assert_eq!(port_pattern(&[1000, 30000]), PortPattern::Random);
        let p = predicted_ports(&PortPattern::Sequential { step: 2, last: 40000 }, 3);
        assert_eq!(p, vec![40002, 40004, 40006]);
        assert_eq!(predicted_ports(&PortPattern::Sequential { step: 1, last: 65534 }, 5), vec![65535]);
        assert!(predicted_ports(&PortPattern::Random, 5).is_empty());
    }

    #[test]
    fn global_v6_filter() {
        assert!(is_global_v6(&"2a01:e0a::1".parse().unwrap()));
        assert!(!is_global_v6(&"fe80::1".parse().unwrap()));
        assert!(!is_global_v6(&"fd00::1".parse().unwrap()));
        assert!(!is_global_v6(&"::1".parse().unwrap()));
    }
}

