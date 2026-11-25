//! One streaming session: a WebRTC peer (str0m), its UDP sockets (IPv4 and,
//! when available, IPv6), and the threads that feed it: capture → encode,
//! audio, and input from the browser.

use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use str0m::bwe::{Bitrate, BweKind};
use str0m::change::SdpOffer;
use str0m::channel::ChannelId;
use str0m::format::Codec;
use str0m::media::{Frequency, MediaKind, MediaTime, Mid, Pt};
use str0m::net::{Protocol, Receive};
use str0m::rtp::Extension;
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc, RtcConfig};

use crate::encoder::{H264Encoder, RateControl};
use crate::input::{self, Injector};
use crate::net;
use crate::source::{self, RawFrame};
use crate::stamp;

#[derive(Clone)]
pub struct Options {
    pub source: String,
    pub width: usize,
    pub height: usize,
    pub fps: u32,
    pub max_bitrate_bps: u32,
    /// Pace packets at the bandwidth estimate and adapt the bitrate.
    pub adaptive: bool,
    pub stun_servers: Vec<String>,
    pub upnp: bool,
    /// Only use this address (LAN test mode); None = discover all paths.
    pub bind: Option<IpAddr>,
    /// Draw the latency stamp on the picture (test mode).
    pub stamp: bool,
}

/// Live numbers for the status page.
#[derive(Clone, Default, serde::Serialize)]
pub struct Status {
    pub state: String,
    pub path: String,
    pub remote: String,
    pub encoder: String,
    pub width: usize,
    pub height: usize,
    pub fps: f64,
    pub bitrate_mbps: f64,
    pub estimate_mbps: f64,
    pub audio: bool,
    pub input_events: u64,
    pub started_secs_ago: u64,
}

enum Msg {
    Net { data: Vec<u8>, source: SocketAddr, destination: SocketAddr },
    Frame { data: Vec<u8>, rtp_90k: u64, captured: Instant },
    Audio { data: Vec<u8>, rtp_48k: u64 },
}

struct Shared {
    stop: AtomicBool,
    connected: AtomicBool,
    want_keyframe: AtomicBool,
    estimate_bps: AtomicU64,
    status: Mutex<Status>,
    started: Instant,
}

impl Shared {
    fn set(&self, f: impl FnOnce(&mut Status)) {
        if let Ok(mut s) = self.status.lock() {
            f(&mut s);
        }
    }
}

pub struct Handle {
    shared: Arc<Shared>,
}

impl Handle {
    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
    pub fn is_done(&self) -> bool {
        self.shared.stop.load(Ordering::Relaxed)
    }
    pub fn status(&self) -> Status {
        let mut s = self.shared.status.lock().map(|s| s.clone()).unwrap_or_default();
        s.started_secs_ago = self.shared.started.elapsed().as_secs();
        s
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Sock {
    socket: UdpSocket,
    local: SocketAddr,
}

/// Accept a browser offer, start the session, and return the SDP answer.
pub fn start(offer_sdp: &str, opts: Options) -> Result<(String, Handle), String> {
    let offer = SdpOffer::from_sdp_string(offer_sdp).map_err(|e| format!("bad offer: {e}"))?;
    let bwe = opts.adaptive.then(|| Bitrate::bps((opts.max_bitrate_bps / 2) as u64));
    let mut rtc = RtcConfig::new()
        .clear_codecs()
        .enable_h264(true)
        .enable_opus(true, false)
        // Not in str0m's default map. Without it the browser keeps a
        // smoothing buffer (measured 9-15 ms) instead of showing frames at once.
        .set_extension(5, Extension::PlayoutDelay)
        .enable_bwe(bwe)
        .build(Instant::now());

    // ---- sockets and candidates: every path at once, ICE picks ----
    let mut socks: Vec<Sock> = Vec::new();
    let mut upnp: Option<net::UpnpMapping> = None;
    let mut offered: Vec<String> = Vec::new();
    let v4 = opts.bind.or_else(net::local_ipv4).unwrap_or(IpAddr::from([127, 0, 0, 1]));
    let s4 = UdpSocket::bind(SocketAddr::new(v4, 0)).map_err(|e| format!("udp bind {v4}: {e}"))?;
    let l4 = s4.local_addr().map_err(|e| e.to_string())?;
    rtc.add_local_candidate(Candidate::host(l4, "udp").map_err(|e| e.to_string())?);
    offered.push(format!("local {l4}"));
    if opts.bind.is_none() {
        // Public address of this socket, for hole punching.
        match net::stun_mapped(&s4, &opts.stun_servers, Duration::from_millis(2500)) {
            Some(mapped) if mapped != l4 => {
                if let Ok(c) = Candidate::server_reflexive(mapped, l4, "udp") {
                    rtc.add_local_candidate(c);
                    offered.push(format!("public {mapped}"));
                }
            }
            Some(_) => {}
            None => offered.push("NO public address (no STUN answer: UDP blocked?)".into()),
        }
        // A real open port on the router, when it allows UPnP.
        if opts.upnp && !l4.ip().is_loopback() {
            if let Some(m) = net::upnp_map(l4) {
                if let Ok(c) = Candidate::server_reflexive(m.external, l4, "udp") {
                    rtc.add_local_candidate(c);
                    offered.push(format!("router port {}", m.external));
                }
                upnp = Some(m);
            }
        }
        // IPv6: no NAT, the address is already public.
        if let Some(v6) = net::global_ipv6() {
            if let Ok(s6) = UdpSocket::bind(SocketAddr::new(v6, 0)) {
                let l6 = s6.local_addr().map_err(|e| e.to_string())?;
                if let Ok(c) = Candidate::host(l6, "udp") {
                    rtc.add_local_candidate(c);
                    offered.push(format!("IPv6 {l6}"));
                    socks.push(Sock { socket: s6, local: l6 });
                }
            }
        }
    }
    socks.insert(0, Sock { socket: s4, local: l4 });

    let answer = rtc.sdp_api().accept_offer(offer).map_err(|e| format!("accept offer: {e}"))?;
    crate::say!("  this PC offers: {}", offered.join(", "));
    if opts.bind.is_none() {
        probe_strict_browser(&mut rtc, offer_sdp);
    }

    let shared = Arc::new(Shared {
        stop: AtomicBool::new(false),
        connected: AtomicBool::new(false),
        want_keyframe: AtomicBool::new(true),
        estimate_bps: AtomicU64::new((opts.max_bitrate_bps / 2) as u64),
        status: Mutex::new(Status { state: "connecting".into(), ..Default::default() }),
        started: Instant::now(),
    });
    let (tx, rx) = mpsc::channel::<Msg>();

    for s in &socks {
        let sock = s.socket.try_clone().map_err(|e| e.to_string())?;
        let (tx, shared) = (tx.clone(), shared.clone());
        thread::Builder::new().name("udp-recv".into()).spawn(move || recv_loop(sock, tx, shared)).map_err(|e| e.to_string())?;
    }
    {
        let (tx, shared, opts) = (tx.clone(), shared.clone(), opts.clone());
        thread::Builder::new().name("encode".into()).spawn(move || encode_loop(opts, tx, shared)).map_err(|e| e.to_string())?;
    }
    {
        let shared = shared.clone();
        thread::Builder::new()
            .name("rtc".into())
            .spawn(move || {
                let _upnp = upnp; // the router port closes when the session ends
                if let Err(e) = rtc_loop(rtc, socks, rx, tx, &shared) {
                    crate::say!("session ended: {e}");
                }
                shared.stop.store(true, Ordering::Relaxed);
                shared.set(|s| s.state = "ended".into());
                crate::say!("session closed");
            })
            .map_err(|e| e.to_string())?;
    }
    Ok((answer.to_sdp_string(), Handle { shared }))
}

/// A browser behind a "strict" NAT (common on phone hotspots and mobile
/// carriers) gets a new public port for every destination, so the port it
/// reported is not the one it uses to reach this PC, and this PC's checks to
/// it are dropped. When those ports count up, the right one is usually among
/// the next few: check them all too. The browser answers on whichever is open.
fn probe_strict_browser(rtc: &mut Rtc, offer_sdp: &str) {
    let seen = net::remote_public_v4(offer_sdp);
    if seen.is_empty() {
        crate::say!("  browser offers: no public IPv4 address (only IPv6 or local)");
    }
    for (ip, ports) in seen {
        let pattern = net::port_pattern(&ports);
        let list = ports.iter().map(u16::to_string).collect::<Vec<_>>().join(",");
        match &pattern {
            net::PortPattern::Stable => crate::say!("  browser offers: public {ip}:{list} (easy network)"),
            net::PortPattern::Sequential { .. } => {
                let probe = net::predicted_ports(&pattern, 40);
                for port in &probe {
                    let a = SocketAddr::new(ip, *port);
                    if let Ok(c) = Candidate::server_reflexive(a, a, "udp") {
                        rtc.add_remote_candidate(c);
                    }
                }
                crate::say!(
                    "  browser offers: public {ip} ports {list} (strict network, ports count up): also trying the next {} ports",
                    probe.len()
                );
            }
            net::PortPattern::Random => crate::say!(
                "  browser offers: public {ip} ports {list} (strict network, random ports): a direct path is unlikely from there"
            ),
        }
    }
}

fn recv_loop(sock: UdpSocket, tx: Sender<Msg>, shared: Arc<Shared>) {
    let destination = sock.local_addr().expect("bound socket");
    let _ = sock.set_read_timeout(Some(Duration::from_millis(500)));
    let mut buf = vec![0u8; 2000];
    while !shared.stop.load(Ordering::Relaxed) {
        match sock.recv_from(&mut buf) {
            Ok((n, source)) => {
                if tx.send(Msg::Net { data: buf[..n].to_vec(), source, destination }).is_err() {
                    return;
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            // Windows reports ICMP "port unreachable" as a receive error; ignore.
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
            Err(e) => {
                crate::say!("udp recv: {e}");
                return;
            }
        }
    }
}

fn encode_loop(opts: Options, tx: Sender<Msg>, shared: Arc<Shared>) {
    let mut src = source::open(&opts.source, opts.width, opts.height, opts.fps);
    let (w, h) = src.size();
    let start_bps = (opts.max_bitrate_bps / 2).max(1_000_000);
    let mut enc = match H264Encoder::new(w, h, opts.fps, start_bps) {
        Ok(e) => e,
        Err(e) => {
            crate::say!("{e}");
            shared.stop.store(true, Ordering::Relaxed);
            return;
        }
    };
    let name = enc.name();
    shared.set(|s| {
        s.encoder = name.into();
        s.width = w;
        s.height = h;
    });
    let mut rc = RateControl::new(1_000_000, opts.max_bitrate_bps);
    let mut frame = RawFrame::new(w, h);
    let period = Duration::from_secs_f64(1.0 / opts.fps as f64);
    let start = Instant::now();
    let mut next = start;
    let mut last_keyframe: Option<Instant> = None;
    let (mut n, mut bytes, mut report) = (0u64, 0usize, Instant::now());

    while !shared.stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if next > now {
            thread::sleep(next - now);
        }
        next = (next + period).max(Instant::now());
        if !shared.connected.load(Ordering::Relaxed) {
            continue;
        }
        if opts.adaptive {
            let est = shared.estimate_bps.load(Ordering::Relaxed);
            if let Some(bps) = rc.decide(enc.bitrate(), est, Instant::now()) {
                match enc.set_bitrate(bps) {
                    Ok(()) => crate::say!("bitrate -> {:.1} Mbit/s (estimate {:.1})", bps as f64 / 1e6, est as f64 / 1e6),
                    Err(e) => crate::say!("{e}"),
                }
            }
        }

        let captured = Instant::now();
        if let Err(e) = src.capture(&mut frame) {
            crate::say!("capture: {e}");
            shared.stop.store(true, Ordering::Relaxed);
            return;
        }
        if opts.stamp {
            let (y, u, v) = frame.planes_mut();
            stamp::write_i420(y, u, v, w, stamp::now_tenth_ms());
        }
        let force = shared.want_keyframe.load(Ordering::Relaxed)
            && last_keyframe.is_none_or(|t| t.elapsed() >= Duration::from_millis(200));
        if force {
            shared.want_keyframe.store(false, Ordering::Relaxed);
        }
        let out = match enc.encode(&frame.i420, force) {
            Ok(o) => o,
            Err(e) => {
                crate::say!("{e}");
                continue;
            }
        };
        if out.data.is_empty() {
            continue;
        }
        if out.keyframe {
            last_keyframe = Some(Instant::now());
        }
        n += 1;
        bytes += out.data.len();
        let rtp_90k = (captured - start).as_micros() as u64 * 9 / 100;
        if tx.send(Msg::Frame { data: out.data, rtp_90k, captured }).is_err() {
            return;
        }
        if report.elapsed() >= Duration::from_secs(2) {
            let secs = report.elapsed().as_secs_f64();
            let est = shared.estimate_bps.load(Ordering::Relaxed);
            shared.set(|s| {
                s.fps = n as f64 / secs;
                s.bitrate_mbps = bytes as f64 * 8.0 / secs / 1e6;
                s.estimate_mbps = est as f64 / 1e6;
            });
            (n, bytes, report) = (0, 0, Instant::now());
        }
    }
}

fn pick_pt(rtc: &mut Rtc, mid: Mid, codec: Codec) -> Option<Pt> {
    let writer = rtc.writer(mid)?;
    let mut best: Option<(u8, Pt)> = None;
    for p in writer.payload_params() {
        if p.spec().codec != codec {
            continue;
        }
        let f = p.spec().format;
        let mut score = 0;
        if codec == Codec::H264 {
            // Packetization mode 1, constrained baseline: what OpenH264 makes.
            if f.packetization_mode == Some(1) {
                score += 2;
            }
            if f.profile_level_id.is_some_and(|id| (id >> 8) == 0x42e0) {
                score += 1;
            }
        }
        if best.is_none_or(|(s, _)| score > s) {
            best = Some((score, p.pt()));
        }
    }
    best.map(|(_, pt)| pt)
}

fn describe_path(local: SocketAddr, remote: SocketAddr) -> String {
    let private = |ip: IpAddr| match ip {
        IpAddr::V4(v) => v.is_private() || v.is_loopback() || v.is_link_local(),
        IpAddr::V6(v) => (v.segments()[0] & 0xfe00) == 0xfc00 || v.is_loopback(),
    };
    match (local.is_ipv6(), private(remote.ip())) {
        (true, _) => "IPv6 direct".into(),
        (false, true) => "same network (LAN)".into(),
        (false, false) => "IPv4 direct (hole punched or router port)".into(),
    }
}

/// How long each frame waits inside str0m (mostly the pacer): from write()
/// to its last RTP packet on the wire. SRTP leaves the RTP header readable.
#[derive(Default)]
struct PaceStats {
    written: VecDeque<Instant>,
    cur_ts: Option<u32>,
    cur_write: Option<Instant>,
    cur_last_tx: Option<Instant>,
    delays_ms: Vec<f64>,
    report: Option<Instant>,
}

impl PaceStats {
    fn on_write(&mut self, at: Instant) {
        self.written.push_back(at);
        if self.written.len() > 120 {
            self.written.clear();
        }
    }
    fn on_transmit(&mut self, pkt: &[u8], video_pt: Pt, now: Instant) {
        if pkt.len() < 12 || pkt[0] >> 6 != 2 || (192..=223).contains(&pkt[1]) || *video_pt != pkt[1] & 0x7f {
            return;
        }
        let ts = u32::from_be_bytes([pkt[4], pkt[5], pkt[6], pkt[7]]);
        if self.cur_ts != Some(ts) {
            self.finish_frame();
            self.cur_ts = Some(ts);
            self.cur_write = self.written.pop_front();
        }
        self.cur_last_tx = Some(now);
    }
    fn finish_frame(&mut self) {
        if let (Some(w), Some(t)) = (self.cur_write.take(), self.cur_last_tx.take()) {
            let ms = t.saturating_duration_since(w).as_secs_f64() * 1000.0;
            if ms < 1000.0 {
                self.delays_ms.push(ms);
            }
        }
    }
    fn maybe_report(&mut self) {
        let r = *self.report.get_or_insert_with(Instant::now);
        if r.elapsed() < Duration::from_secs(10) || self.delays_ms.is_empty() {
            return;
        }
        let mut d = std::mem::take(&mut self.delays_ms);
        d.sort_by(|a, b| a.total_cmp(b));
        let q = |p: f64| d[((d.len() - 1) as f64 * p) as usize];
        crate::say!("send queue: p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms", q(0.5), q(0.95), q(1.0));
        self.report = Some(Instant::now());
    }
}

fn rtc_loop(mut rtc: Rtc, socks: Vec<Sock>, rx: Receiver<Msg>, tx: Sender<Msg>, shared: &Arc<Shared>) -> Result<(), String> {
    let mut video: Option<(Mid, Pt)> = None;
    let mut audio: Option<(Mid, Pt)> = None;
    // Keeps audio capture alive for the session.
    let mut _audio_capture: Option<Box<dyn std::any::Any>> = None;
    let mut pace = PaceStats::default();
    let mut injector = Injector::new();
    let mut input_channels: Vec<ChannelId> = Vec::new();
    let mut path_known = false;
    let deadline_connect = Instant::now() + Duration::from_secs(30);

    loop {
        if shared.stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        let timeout = match rtc.poll_output().map_err(|e| e.to_string())? {
            Output::Timeout(t) => t,
            Output::Transmit(t) => {
                let sock = socks.iter().find(|s| s.local == t.source).unwrap_or(&socks[0]);
                let _ = sock.socket.send_to(&t.contents, t.destination);
                if let Some((_, pt)) = video {
                    pace.on_transmit(&t.contents, pt, Instant::now());
                }
                continue;
            }
            Output::Event(ev) => {
                match ev {
                    Event::IceConnectionStateChange(s) => match s {
                        IceConnectionState::Connected | IceConnectionState::Completed => {
                            shared.want_keyframe.store(true, Ordering::Relaxed);
                            shared.connected.store(true, Ordering::Relaxed);
                            shared.set(|st| st.state = "streaming".into());
                        }
                        IceConnectionState::Disconnected if !shared.connected.load(Ordering::Relaxed) => {
                            return Err("no working path between the two networks (see the lines above)".into())
                        }
                        IceConnectionState::Disconnected => return Ok(()),
                        _ => {}
                    },
                    Event::MediaAdded(m) if m.kind == MediaKind::Video => {
                        video = pick_pt(&mut rtc, m.mid, Codec::H264).map(|pt| (m.mid, pt));
                    }
                    Event::MediaAdded(m) if m.kind == MediaKind::Audio => {
                        audio = pick_pt(&mut rtc, m.mid, Codec::Opus).map(|pt| (m.mid, pt));
                        if audio.is_some() {
                            let (atx, arx) = mpsc::channel::<crate::audio::AudioPacket>();
                            match crate::audio::start(atx, Arc::new(AtomicBool::new(false))) {
                                Ok(g) => {
                                    _audio_capture = Some(g);
                                    let tx = tx.clone();
                                    thread::spawn(move || {
                                        for p in arx {
                                            if tx.send(Msg::Audio { data: p.data, rtp_48k: p.rtp_48k }).is_err() {
                                                return;
                                            }
                                        }
                                    });
                                    shared.set(|st| st.audio = true);
                                }
                                Err(e) => crate::say!("no audio: {e}"),
                            }
                        }
                    }
                    Event::ChannelOpen(id, label) => {
                        if label == "fast" || label == "reliable" {
                            input_channels.push(id);
                        }
                    }
                    Event::ChannelData(d) => {
                        if input_channels.contains(&d.id) {
                            if let Some(ev) = input::decode(&d.data) {
                                injector.handle(ev);
                            }
                        }
                    }
                    Event::KeyframeRequest(_) => shared.want_keyframe.store(true, Ordering::Relaxed),
                    Event::EgressBitrateEstimate(BweKind::Twcc { estimate, .. })
                    | Event::EgressBitrateEstimate(BweKind::Remb { estimate, .. }) => {
                        shared.estimate_bps.store(estimate.as_u64(), Ordering::Relaxed);
                    }
                    _ => {}
                }
                continue;
            }
        };

        if !shared.connected.load(Ordering::Relaxed) && Instant::now() > deadline_connect {
            return Err("no connection within 30 s (no working path between the two networks)".into());
        }

        let wait = timeout.saturating_duration_since(Instant::now());
        let msg = match rx.recv_timeout(wait) {
            Ok(m) => m,
            Err(RecvTimeoutError::Timeout) => {
                rtc.handle_input(Input::Timeout(Instant::now())).map_err(|e| e.to_string())?;
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        };
        match msg {
            Msg::Net { data, source, destination } => {
                let Ok(contents) = data.as_slice().try_into() else { continue };
                let input = Input::Receive(Instant::now(), Receive { proto: Protocol::Udp, source, destination, contents });
                if rtc.accepts(&input) {
                    rtc.handle_input(input).map_err(|e| e.to_string())?;
                    if shared.connected.load(Ordering::Relaxed) && !path_known {
                        path_known = true;
                        let p = describe_path(destination, source);
                        crate::say!("connected: {p}");
                        shared.set(|st| {
                            st.path = p;
                            st.remote = source.to_string();
                        });
                    }
                }
            }
            Msg::Frame { data, rtp_90k, captured } => {
                let Some((mid, pt)) = video else { continue };
                let Some(writer) = rtc.writer(mid) else { continue };
                let zero = MediaTime::from_millis(0);
                match writer.playout_delay(zero, zero).write(pt, captured, MediaTime::from_90khz(rtp_90k), data) {
                    Ok(()) => pace.on_write(Instant::now()),
                    Err(e) => crate::say!("write video: {e}"),
                }
                pace.maybe_report();
                let n = injector.injected;
                shared.set(|st| st.input_events = n);
            }
            Msg::Audio { data, rtp_48k } => {
                let Some((mid, pt)) = audio else { continue };
                let Some(writer) = rtc.writer(mid) else { continue };
                let _ = writer.write(pt, Instant::now(), MediaTime::new(rtp_48k, Frequency::FORTY_EIGHT_KHZ), data);
            }
        }
    }
}
