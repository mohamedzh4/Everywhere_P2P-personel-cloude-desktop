//! Everywhere host: use this PC from any browser, anywhere, with no server.
//!
//!   everywhere-host                 run: meeting point + status page
//!   everywhere-host lan [options]   LAN test mode (phase 1): the browser loads
//!                                   a test page from this PC, no relays
//!   everywhere-host show            print the PC code, password, settings file
//!   everywhere-host new-password    make a new password
//!
//! LAN options: --port 8080 --size 1280x720 --fps 60 --bitrate 8 --bind IP
//!              --no-pacing --source pattern|screen

mod audio;
mod config;
mod encoder;
mod input;
mod net;
mod nostr;
mod pake;
mod session;
mod signaling;
mod source;
mod stamp;
mod status;

use std::io::Read;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use tiny_http::{Header, Method, Response, Server};

const LAN_HTML: &str = include_str!("../web/index.html");
const LAN_JS: &str = include_str!("../web/client.js");

fn main() {
    str0m::crypto::from_feature_flags().install_process_default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        None | Some("run") => run(),
        Some("lan") => lan(&args[1..]),
        Some("show") => show(),
        Some("new-password") => new_password(),
        Some("-h" | "--help" | "help") => {
            println!("{HELP}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command {other}\n\n{HELP}")),
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

const HELP: &str = "everywhere-host [run | lan | show | new-password]

  run            (default) wait for browsers through the meeting point
  lan            LAN test mode: open the printed URL on the same network
  show           print the PC code, password and settings file
  new-password   make a new password (the old one stops working)";

fn show() -> Result<(), String> {
    let c = config::load_or_create()?;
    println!("PC code:   {}", config::pretty_code(&c.pc_code));
    println!("Password:  {}", c.password);
    println!("Web page:  {}", c.web_url);
    println!("Settings:  {}", config::path().display());
    Ok(())
}

fn new_password() -> Result<(), String> {
    let mut c = config::load_or_create()?;
    c.password = config::new_password();
    config::save(&c)?;
    println!("New password: {}  (restart the host if it is running)", c.password);
    Ok(())
}

fn session_options(c: &config::Config) -> session::Options {
    session::Options {
        source: c.source.clone(),
        width: c.width,
        height: c.height,
        fps: c.fps,
        max_bitrate_bps: (c.max_bitrate_mbps * 1e6) as u32,
        adaptive: true,
        stun_servers: c.stun_servers.clone(),
        upnp: c.upnp,
        bind: None,
        stamp: false,
    }
}

fn run() -> Result<(), String> {
    let c = config::load_or_create()?;
    let opts = session_options(&c);
    let status_port = c.status_port;
    println!("Everywhere host");
    println!();
    println!("  On any laptop open  {}", c.web_url);
    println!("  PC code             {}", config::pretty_code(&c.pc_code));
    println!("  Password            {}", c.password);
    println!();
    println!("  Status page (this PC only): http://127.0.0.1:{status_port}");
    println!();
    println!("  Network: {}", net::home_report(&c.stun_servers));
    println!();
    let cfg = Arc::new(Mutex::new(c));
    let state = Arc::new(Mutex::new(signaling::HostState::default()));
    {
        let (cfg, state) = (cfg.clone(), state.clone());
        std::thread::Builder::new()
            .name("status".into())
            .spawn(move || status::run(status_port, cfg, state))
            .map_err(|e| e.to_string())?;
    }
    signaling::run(cfg, state, opts);
    Ok(())
}

// ---------------- LAN test mode (phase 1) ----------------

fn lan(args: &[String]) -> Result<(), String> {
    let c = config::load_or_create()?;
    let mut o = session_options(&c);
    o.source = "pattern".into();
    o.width = 1280;
    o.height = 720;
    o.max_bitrate_bps = 8_000_000;
    o.upnp = false;
    o.stamp = true;
    let mut port: u16 = 8080;
    let mut it = args.iter();
    while let Some(k) = it.next() {
        let mut v = || it.next().cloned().ok_or(format!("{k} needs a value"));
        match k.as_str() {
            "--port" => port = v()?.parse().map_err(|_| "bad --port")?,
            "--fps" => o.fps = v()?.parse().map_err(|_| "bad --fps")?,
            "--bitrate" => o.max_bitrate_bps = (v()?.parse::<f64>().map_err(|_| "bad --bitrate")? * 1e6) as u32,
            "--bind" => o.bind = Some(v()?.parse::<IpAddr>().map_err(|_| "bad --bind")?),
            "--source" => o.source = v()?,
            "--no-pacing" => o.adaptive = false,
            "--size" => {
                let s = v()?;
                let (w, h) = s.split_once('x').ok_or("--size is WIDTHxHEIGHT")?;
                o.width = w.parse().map_err(|_| "bad width")?;
                o.height = h.parse().map_err(|_| "bad height")?;
                if o.width % 2 != 0 || o.height % 2 != 0 || o.width < 640 {
                    return Err("--size must be even and at least 640 wide".into());
                }
            }
            _ => return Err(format!("unknown option {k}")),
        }
    }
    // LAN mode: only the local address (no STUN/UPnP).
    let ip = o.bind.or_else(net::local_ipv4).unwrap_or(IpAddr::from([127, 0, 0, 1]));
    o.bind = Some(ip);
    let token = hex::encode(pake::random_bytes::<16>());
    let server = Server::http(("0.0.0.0", port)).map_err(|e| format!("cannot listen on port {port}: {e}"))?;
    println!("Everywhere host, LAN test mode");
    println!("Open in a browser on this network:\n\n    http://{ip}:{port}/?t={token}\n");
    println!(
        "Stream: {} {}x{} @ {} fps, up to {:.0} Mbit/s, pacing {}",
        o.source,
        o.width,
        o.height,
        o.fps,
        o.max_bitrate_bps as f64 / 1e6,
        if o.adaptive { "on" } else { "off" }
    );

    let mut current: Option<session::Handle> = None;
    for mut req in server.incoming_requests() {
        let url = req.url().to_string();
        let (path, query) = url.split_once('?').unwrap_or((&url, ""));
        let authorized = query.split('&').any(|kv| kv == format!("t={token}"));
        let respond = |req: tiny_http::Request, code: u16, ctype: &str, body: String| {
            let h = Header::from_bytes("Content-Type", ctype).expect("header");
            let nc = Header::from_bytes("Cache-Control", "no-store").expect("header");
            let _ = req.respond(Response::from_string(body).with_status_code(code).with_header(h).with_header(nc));
        };
        if !authorized {
            respond(req, 403, "text/plain", "missing or wrong token\n".into());
            continue;
        }
        match (req.method(), path) {
            (Method::Get, "/") => {
                let html = LAN_HTML.replace("src=\"client.js\"", &format!("src=\"client.js?t={token}\""));
                respond(req, 200, "text/html; charset=utf-8", html)
            }
            (Method::Get, "/client.js") => respond(req, 200, "text/javascript", LAN_JS.into()),
            (Method::Post, "/offer") => {
                let mut sdp = String::new();
                if req.as_reader().take(64 * 1024).read_to_string(&mut sdp).is_err() {
                    respond(req, 400, "text/plain", "bad body\n".into());
                    continue;
                }
                if let Some(old) = current.take() {
                    old.stop();
                }
                match session::start(&sdp, o.clone()) {
                    Ok((answer, handle)) => {
                        println!("viewer connected from {:?}", req.remote_addr());
                        current = Some(handle);
                        respond(req, 200, "application/sdp", answer);
                    }
                    Err(e) => {
                        eprintln!("offer failed: {e}");
                        respond(req, 400, "text/plain", format!("{e}\n"));
                    }
                }
            }
            _ => respond(req, 404, "text/plain", "not found\n".into()),
        }
    }
    Ok(())
}
