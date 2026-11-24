//! The host side of the meeting point: listen on the relays for this PC's
//! topic, run the password handshake, and answer a proven browser's WebRTC
//! offer.
//!
//! Messages (JSON in the Nostr event content, binary fields base64):
//!   B→H hello  {v:1, t:"hello",  sid, ya}
//!   H→B ack    {v:1, t:"ack",    sid, yb, tag}          tag = tag_h
//!   B→H offer  {v:1, t:"offer",  sid, tag, sdp}         tag = tag_b, sdp sealed with k_b2h
//!   H→B answer {v:1, t:"answer", sid, sdp}              sdp sealed with k_h2b
//!   H→B error  {v:1, t:"error",  sid, reason, wait?}    reason: locked | wait | failed
//!
//! Guessing: every hello counts as a failed attempt until a correct `tag_b`
//! arrives. After 5 failures in a row each new hello must wait 1, 2, 4 … s
//! (max 1 h); after 20 failures in a day the PC stops answering until
//! "unlock" on the status page.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde_json::{json, Value};

use crate::config::Config;
use crate::nostr::{self, Keys, RelayPool, KIND};
use crate::pake;
use crate::session;

pub const MAX_FAILURES_PER_DAY: u32 = 20;
const FREE_FAILURES: u32 = 5;

#[derive(Debug, Clone, serde::Serialize)]
pub struct Guard {
    pub failures_in_row: u32,
    pub failures_today: u32,
    pub locked: bool,
    #[serde(skip)]
    pub next_allowed: Option<Instant>,
    #[serde(skip)]
    day_start: Instant,
    pub last_attempt_secs_ago: Option<u64>,
    #[serde(skip)]
    last_attempt: Option<Instant>,
}

impl Default for Guard {
    fn default() -> Self {
        Self {
            failures_in_row: 0,
            failures_today: 0,
            locked: false,
            next_allowed: None,
            day_start: Instant::now(),
            last_attempt_secs_ago: None,
            last_attempt: None,
        }
    }
}

pub enum Verdict {
    Go,
    Locked,
    Wait(u64),
}

impl Guard {
    /// Called for each hello. Counts it as a failure until proven otherwise.
    pub fn attempt(&mut self, now: Instant) -> Verdict {
        if now.duration_since(self.day_start) > Duration::from_secs(86_400) {
            self.day_start = now;
            self.failures_today = 0;
        }
        if self.locked {
            return Verdict::Locked;
        }
        if let Some(t) = self.next_allowed {
            if now < t {
                return Verdict::Wait((t - now).as_secs() + 1);
            }
        }
        self.last_attempt = Some(now);
        self.failures_in_row += 1;
        self.failures_today += 1;
        if self.failures_today >= MAX_FAILURES_PER_DAY {
            self.locked = true;
        }
        if self.failures_in_row >= FREE_FAILURES {
            let secs = 1u64 << (self.failures_in_row - FREE_FAILURES).min(12);
            self.next_allowed = Some(now + Duration::from_secs(secs.min(3600)));
        }
        Verdict::Go
    }

    pub fn success(&mut self) {
        self.failures_in_row = 0;
        self.failures_today = self.failures_today.saturating_sub(1);
        self.next_allowed = None;
    }

    pub fn unlock(&mut self) {
        *self = Guard::default();
    }

    pub fn snapshot(&self) -> Guard {
        let mut g = self.clone();
        g.last_attempt_secs_ago = self.last_attempt.map(|t| t.elapsed().as_secs());
        g
    }
}

/// Everything the status page shows and controls.
#[derive(Default)]
pub struct HostState {
    pub session: Option<session::Handle>,
    pub guard: Guard,
    pub relays_up: usize,
    pub relays_total: usize,
    pub events: Vec<String>,
}

impl HostState {
    pub fn log(&mut self, msg: impl Into<String>) {
        let line = format!("{} {}", clock(), msg.into());
        println!("{line}");
        self.events.push(line);
        if self.events.len() > 50 {
            self.events.remove(0);
        }
    }
}

fn clock() -> String {
    let s = nostr::now_secs() % 86_400;
    format!("{:02}:{:02}:{:02} UTC", s / 3600, s / 60 % 60, s % 60)
}

struct Pending {
    browser: String,
    keys: pake::SessionKeys,
    created: Instant,
}

fn b64(v: &Value, k: &str) -> Option<Vec<u8>> {
    B64.decode(v.get(k)?.as_str()?).ok()
}

/// Run the meeting point forever (call from its own thread).
pub fn run(cfg: Arc<Mutex<Config>>, state: Arc<Mutex<HostState>>, opts: session::Options) {
    let (relays, code) = {
        let c = cfg.lock().expect("config");
        (c.relays.clone(), c.pc_code.clone())
    };
    let topic = nostr::topic_for(&code);
    let keys = Keys::random();
    let (pool, rx) = RelayPool::start(&relays, &topic);
    if let Ok(mut s) = state.lock() {
        s.relays_total = relays.len();
    }
    let mut pending: HashMap<Vec<u8>, Pending> = HashMap::new();

    let send = |to: &str, body: Value| {
        let tags = vec![vec!["t".to_string(), topic.clone()], vec!["p".to_string(), to.to_string()]];
        pool.publish(&keys.sign(KIND, tags, body.to_string(), nostr::now_secs()));
    };

    loop {
        let ev = rx.recv_timeout(Duration::from_secs(1));
        if let Ok(mut s) = state.lock() {
            s.relays_up = pool.connected();
            if s.session.as_ref().is_some_and(|h| h.is_done()) {
                s.session = None;
                s.log("session ended");
            }
        }
        pending.retain(|_, p| p.created.elapsed() < Duration::from_secs(60));
        let Ok(ev) = ev else { continue };
        if ev.pubkey == keys.pubkey {
            continue; // our own message, echoed by the relay
        }
        let Ok(msg) = serde_json::from_str::<Value>(&ev.content) else { continue };
        if msg.get("v").and_then(Value::as_u64) != Some(1) {
            continue;
        }
        let Some(sid) = b64(&msg, "sid").filter(|s| s.len() == pake::SID_LEN) else { continue };
        let sid_b64 = B64.encode(&sid);

        match msg.get("t").and_then(Value::as_str) {
            Some("hello") => {
                let Some(ya) = b64(&msg, "ya") else { continue };
                if pending.len() > 32 {
                    continue; // flood: ignore until old ones expire
                }
                let verdict = state.lock().map(|mut s| s.guard.attempt(Instant::now())).unwrap_or(Verdict::Locked);
                match verdict {
                    Verdict::Locked => {
                        send(&ev.pubkey, json!({"v":1,"t":"error","sid":sid_b64,"reason":"locked"}));
                        continue;
                    }
                    Verdict::Wait(secs) => {
                        send(&ev.pubkey, json!({"v":1,"t":"error","sid":sid_b64,"reason":"wait","wait":secs}));
                        continue;
                    }
                    Verdict::Go => {}
                }
                let (code, password) = {
                    let c = cfg.lock().expect("config");
                    (c.pc_code.clone(), c.password.clone())
                };
                let Some((yb, k)) = pake::respond(&code, &password, &sid, &ya, &pake::random_bytes()) else { continue };
                send(&ev.pubkey, json!({"v":1,"t":"ack","sid":sid_b64,"yb":B64.encode(yb),"tag":B64.encode(k.tag_h)}));
                pending.insert(sid, Pending { browser: ev.pubkey.clone(), keys: k, created: Instant::now() });
            }
            Some("offer") => {
                let Some(p) = pending.get(&sid) else { continue };
                if p.browser != ev.pubkey {
                    continue; // not the browser that said hello
                }
                let tag_ok = b64(&msg, "tag").is_some_and(|t| pake::tags_equal(&t, &p.keys.tag_b));
                if !tag_ok {
                    if let Ok(mut s) = state.lock() {
                        s.log("wrong password attempt");
                    }
                    pending.remove(&sid);
                    continue;
                }
                let p = pending.remove(&sid).expect("checked above");
                let sdp = b64(&msg, "sdp").and_then(|s| pake::open(&p.keys.k_b2h, &pake::aad("b2h", &sid), &s));
                let Some(sdp) = sdp.and_then(|s| String::from_utf8(s).ok()) else { continue };
                {
                    let mut s = state.lock().expect("state");
                    s.guard.success();
                    // One viewer at a time: a new proven browser replaces the old one.
                    if let Some(old) = s.session.take() {
                        old.stop();
                        s.log("previous session replaced");
                    }
                }
                match session::start(&sdp, opts.clone()) {
                    Ok((answer, handle)) => {
                        let sealed = pake::seal(&p.keys.k_h2b, &pake::aad("h2b", &sid), answer.as_bytes(), pake::random_bytes());
                        send(&ev.pubkey, json!({"v":1,"t":"answer","sid":sid_b64,"sdp":B64.encode(sealed)}));
                        if let Ok(mut s) = state.lock() {
                            s.session = Some(handle);
                            s.log("browser connected with the right password");
                        }
                    }
                    Err(e) => {
                        send(&ev.pubkey, json!({"v":1,"t":"error","sid":sid_b64,"reason":"failed"}));
                        if let Ok(mut s) = state.lock() {
                            s.log(format!("could not start session: {e}"));
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_backs_off_and_locks() {
        let mut g = Guard::default();
        let t = Instant::now();
        for _ in 0..FREE_FAILURES - 1 {
            assert!(matches!(g.attempt(t), Verdict::Go));
        }
        assert!(matches!(g.attempt(t), Verdict::Go)); // 5th: now delays start
        assert!(matches!(g.attempt(t), Verdict::Wait(_)));
        let mut later = t + Duration::from_secs(2);
        for _ in 0..30 {
            if let Verdict::Locked = g.attempt(later) {
                break;
            }
            later += Duration::from_secs(4000);
        }
        assert!(g.locked, "locks after {MAX_FAILURES_PER_DAY} failures");
        g.unlock();
        assert!(matches!(g.attempt(later), Verdict::Go));
    }

    #[test]
    fn success_resets_backoff() {
        let mut g = Guard::default();
        let t = Instant::now();
        for _ in 0..6 {
            let _ = g.attempt(t);
        }
        g.success();
        assert!(matches!(g.attempt(t), Verdict::Go));
    }
}
