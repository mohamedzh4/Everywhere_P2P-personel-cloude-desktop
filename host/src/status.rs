//! Local status page: http://127.0.0.1:47800 (this PC only).
//!
//! Shows the PC code and password, relay connections, the live session, and
//! wrong-password attempts; buttons to disconnect, unlock and make a new
//! password. Stands in for the tray app until phase 6.
//!
//! Safety: bound to 127.0.0.1. Requests must carry `Host: 127.0.0.1:<port>` or
//! `localhost:<port>` (blocks DNS rebinding) and actions need the header
//! `X-Everywhere: 1`, which a web page from elsewhere cannot send without a
//! CORS preflight that we never approve (blocks cross-site form posts).

use std::sync::{Arc, Mutex};

use serde_json::json;
use tiny_http::{Header, Method, Response, Server};

use crate::config::{self, Config};
use crate::signaling::HostState;

const PAGE: &str = include_str!("../web/status.html");

const ICON: &[u8] = include_bytes!("../../assets/icons/everywhere-64.png");
const FONT: &[u8] = include_bytes!("../../web/vendor/fonts/figtree-latin-wght-normal.woff2");

/// Serve the page on an already bound 127.0.0.1 server.
pub fn run(server: Server, cfg: Arc<Mutex<Config>>, state: Arc<Mutex<HostState>>) {
    let port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(0);
    let allowed_hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    for mut req in server.incoming_requests() {
        let host_ok = req
            .headers()
            .iter()
            .any(|h| h.field.equiv("Host") && allowed_hosts.iter().any(|a| h.value.as_str() == a));
        let marked = req.headers().iter().any(|h| h.field.equiv("X-Everywhere") && h.value.as_str() == "1");
        let reply = |req: tiny_http::Request, code: u16, ctype: &str, body: String| {
            let _ = req.respond(
                Response::from_string(body)
                    .with_status_code(code)
                    .with_header(Header::from_bytes("Content-Type", ctype).expect("header"))
                    .with_header(Header::from_bytes("Cache-Control", "no-store").expect("header"))
                    .with_header(Header::from_bytes("X-Frame-Options", "DENY").expect("header"))
                    .with_header(Header::from_bytes("Referrer-Policy", "no-referrer").expect("header")),
            );
        };
        if !host_ok {
            reply(req, 403, "text/plain", "forbidden\n".into());
            continue;
        }
        let url = req.url().to_string();
        match (req.method().clone(), url.as_str()) {
            (Method::Get, u) if u == "/" || u.starts_with("/?") => reply(req, 200, "text/html; charset=utf-8", PAGE.into()),
            (Method::Get, "/icon.png") => {
                let _ = req.respond(
                    Response::from_data(ICON)
                        .with_header(Header::from_bytes("Content-Type", "image/png").expect("header"))
                        .with_header(Header::from_bytes("Cache-Control", "max-age=86400").expect("header")),
                );
            }
            (Method::Get, "/figtree.woff2") => {
                let _ = req.respond(
                    Response::from_data(FONT)
                        .with_header(Header::from_bytes("Content-Type", "font/woff2").expect("header"))
                        .with_header(Header::from_bytes("Cache-Control", "max-age=86400").expect("header")),
                );
            }
            (Method::Post, "/api/open-web") if marked => {
                crate::ui::open_in_browser(&cfg.lock().expect("config").web_url.clone());
                reply(req, 200, "application/json", "{}".into());
            }
            (Method::Get, "/api/status") if marked => {
                let c = cfg.lock().expect("config").clone();
                let s = state.lock().expect("state");
                let session = s.session.as_ref().map(|h| h.status());
                let body = json!({
                    "pc_code": config::pretty_code(&c.pc_code),
                    "password": c.password,
                    "web_url": c.web_url,
                    "relays_up": s.relays_up,
                    "relays_total": s.relays_total,
                    "network": s.network,
                    "paused": s.paused,
                    "checks": s.checks,
                    "version": env!("CARGO_PKG_VERSION"),
                    "guard": s.guard.snapshot(),
                    "session": session,
                    "events": crate::logbook::recent(),
                    "config_path": config::path().display().to_string(),
                });
                reply(req, 200, "application/json", body.to_string());
            }
            (Method::Post, "/api/disconnect") if marked => {
                let mut s = state.lock().expect("state");
                if let Some(h) = s.session.take() {
                    h.stop();
                    s.log("disconnected from the status page");
                }
                reply(req, 200, "application/json", "{}".into());
            }
            (Method::Post, u) if marked && u.starts_with("/api/open-settings?which=") => {
                crate::permissions::open_settings(&u["/api/open-settings?which=".len()..]);
                reply(req, 200, "application/json", "{}".into());
            }
            (Method::Post, "/api/stop-sharing") if marked => {
                let mut s = state.lock().expect("state");
                s.paused = true;
                if let Some(h) = s.session.take() {
                    h.stop();
                }
                s.log("sharing stopped: nobody can connect");
                reply(req, 200, "application/json", "{}".into());
            }
            (Method::Post, "/api/start-sharing") if marked => {
                let mut s = state.lock().expect("state");
                s.paused = false;
                s.log("sharing started");
                reply(req, 200, "application/json", "{}".into());
            }
            (Method::Post, "/api/unlock") if marked => {
                let mut s = state.lock().expect("state");
                s.guard.unlock();
                s.log("unlocked from the status page");
                reply(req, 200, "application/json", "{}".into());
            }
            (Method::Post, "/api/new-password") if marked => {
                let mut c = cfg.lock().expect("config");
                c.password = config::new_password();
                let saved = config::save(&c);
                let mut s = state.lock().expect("state");
                // A new password also ends any session that used the old one.
                if let Some(h) = s.session.take() {
                    h.stop();
                }
                s.guard.unlock();
                s.log("new password made");
                match saved {
                    Ok(()) => reply(req, 200, "application/json", "{}".into()),
                    Err(e) => reply(req, 500, "text/plain", e),
                }
            }
            _ => {
                let mut sink = String::new();
                let _ = std::io::Read::read_to_string(&mut req.as_reader(), &mut sink);
                reply(req, 404, "text/plain", "not found\n".into());
            }
        }
    }
}
