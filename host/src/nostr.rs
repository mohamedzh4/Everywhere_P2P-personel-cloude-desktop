//! Meeting point over public Nostr relays (NIP-01).
//!
//! The relays only carry short "hello" messages between the browser and this
//! PC. They see a topic hash and opaque sealed bytes; the security comes from
//! the password handshake in `pake.rs`, not from Nostr. Each run uses a fresh
//! random Nostr key, so nothing links sessions together.
//!
//! Events are kind 25050 (ephemeral range: relays forward them and do not
//! store them), tagged `["t", topic]` with topic = hex SHA-256 of
//! "everywhere/v1/" + PC code.

use std::collections::HashSet;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use k256::schnorr::{Signature, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

pub const KIND: u32 = 25050;

/// Shipped relay list. Several, so one going down does not matter.
pub const DEFAULT_RELAYS: &[&str] = &[
    "wss://relay.damus.io",
    "wss://nos.lol",
    "wss://relay.primal.net",
    "wss://nostr.mom",
    "wss://relay.nostr.band",
];

pub fn topic_for(pc_code: &str) -> String {
    hex::encode(Sha256::digest(format!("everywhere/v1/{pc_code}").as_bytes()))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    pub id: String,
    pub pubkey: String,
    pub created_at: u64,
    pub kind: u32,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub sig: String,
}

fn event_id(pubkey: &str, created_at: u64, kind: u32, tags: &[Vec<String>], content: &str) -> [u8; 32] {
    let ser = json!([0, pubkey, created_at, kind, tags, content]).to_string();
    Sha256::digest(ser.as_bytes()).into()
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

pub struct Keys {
    sk: SigningKey,
    pub pubkey: String,
}

impl Keys {
    pub fn random() -> Self {
        loop {
            let b: [u8; 32] = crate::pake::random_bytes();
            if let Ok(k) = Self::from_secret(&b) {
                return k;
            }
        }
    }

    pub fn from_secret(b: &[u8; 32]) -> Result<Self, String> {
        let sk = SigningKey::from_slice(b).map_err(|e| e.to_string())?;
        let pubkey = hex::encode(sk.verifying_key().to_bytes());
        Ok(Self { sk, pubkey })
    }

    pub fn sign(&self, kind: u32, tags: Vec<Vec<String>>, content: String, created_at: u64) -> Event {
        let id = event_id(&self.pubkey, created_at, kind, &tags, &content);
        let aux: [u8; 32] = crate::pake::random_bytes();
        let sig: Signature = self.sk.sign_raw(&id, &aux).expect("BIP-340 signing");
        Event { id: hex::encode(id), pubkey: self.pubkey.clone(), created_at, kind, tags, content, sig: hex::encode(sig.to_bytes()) }
    }
}

pub fn verify(ev: &Event) -> bool {
    let id = event_id(&ev.pubkey, ev.created_at, ev.kind, &ev.tags, &ev.content);
    if hex::encode(id) != ev.id {
        return false;
    }
    let (Ok(pk), Ok(sig)) = (hex::decode(&ev.pubkey), hex::decode(&ev.sig)) else { return false };
    let (Ok(vk), Ok(sig)) = (VerifyingKey::from_slice(&pk), Signature::try_from(sig.as_slice())) else { return false };
    vk.verify_raw(&id, &sig).is_ok()
}

/// A set of relay connections, one thread each, with reconnects.
pub struct RelayPool {
    outboxes: Vec<Sender<String>>,
    stop: Arc<AtomicBool>,
    pub status: Arc<Mutex<Vec<(String, bool)>>>,
}

impl RelayPool {
    /// Connect to `relays` and subscribe to events of our kind with `topic`.
    /// Verified, de-duplicated events arrive on the returned receiver.
    pub fn start(relays: &[String], topic: &str) -> (Self, Receiver<Event>) {
        let (in_tx, in_rx) = mpsc::channel::<Event>();
        let seen = Arc::new(Mutex::new(HashSet::<String>::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(relays.iter().map(|r| (r.clone(), false)).collect::<Vec<_>>()));
        let mut outboxes = Vec::new();
        for (i, url) in relays.iter().enumerate() {
            let (out_tx, out_rx) = mpsc::channel::<String>();
            outboxes.push(out_tx);
            let (url, topic, in_tx, seen, stop, status) =
                (url.clone(), topic.to_string(), in_tx.clone(), seen.clone(), stop.clone(), status.clone());
            thread::Builder::new()
                .name(format!("relay-{i}"))
                .spawn(move || relay_thread(i, url, topic, out_rx, in_tx, seen, stop, status))
                .expect("spawn relay thread");
        }
        (Self { outboxes, stop, status }, in_rx)
    }

    pub fn publish(&self, ev: &Event) {
        let msg = json!(["EVENT", ev]).to_string();
        for o in &self.outboxes {
            let _ = o.send(msg.clone());
        }
    }

    pub fn connected(&self) -> usize {
        self.status.lock().map(|s| s.iter().filter(|(_, up)| *up).count()).unwrap_or(0)
    }
}

impl Drop for RelayPool {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

type Ws = WebSocket<MaybeTlsStream<TcpStream>>;

fn set_timeout(ws: &mut Ws, d: Duration) {
    let _ = match ws.get_mut() {
        MaybeTlsStream::Plain(s) => s.set_read_timeout(Some(d)),
        MaybeTlsStream::NativeTls(s) => s.get_mut().set_read_timeout(Some(d)),
        _ => Ok(()),
    };
}

#[allow(clippy::too_many_arguments)]
fn relay_thread(
    index: usize,
    url: String,
    topic: String,
    outbox: Receiver<String>,
    inbox: Sender<Event>,
    seen: Arc<Mutex<HashSet<String>>>,
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<Vec<(String, bool)>>>,
) {
    let mut backoff = Duration::from_secs(1);
    let set_up = |up: bool| {
        if let Ok(mut s) = status.lock() {
            s[index].1 = up;
        }
    };
    while !stop.load(Ordering::Relaxed) {
        let mut ws = match tungstenite::connect(url.as_str()) {
            Ok((ws, _)) => ws,
            Err(_) => {
                thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_secs(60));
                continue;
            }
        };
        set_timeout(&mut ws, Duration::from_millis(100));
        let sub = json!(["REQ", "everywhere", {"kinds": [KIND], "#t": [topic], "since": now_secs().saturating_sub(10)}]);
        if ws.send(Message::text(sub.to_string())).is_err() {
            continue;
        }
        set_up(true);
        backoff = Duration::from_secs(1);
        let mut last_ping = Instant::now();
        loop {
            if stop.load(Ordering::Relaxed) {
                let _ = ws.close(None);
                set_up(false);
                return;
            }
            while let Ok(m) = outbox.try_recv() {
                let _ = ws.send(Message::text(m));
            }
            if last_ping.elapsed() > Duration::from_secs(30) {
                let _ = ws.send(Message::Ping(Vec::new().into()));
                last_ping = Instant::now();
            }
            match ws.read() {
                Ok(Message::Text(t)) => {
                    if let Some(ev) = parse_event(t.as_str(), &topic) {
                        let fresh = seen.lock().map(|mut s| s.insert(ev.id.clone())).unwrap_or(false);
                        if fresh && inbox.send(ev).is_err() {
                            return;
                        }
                    }
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(_) => break,
            }
        }
        set_up(false);
        thread::sleep(backoff);
    }
}

fn parse_event(text: &str, topic: &str) -> Option<Event> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let arr = v.as_array()?;
    if arr.first()?.as_str()? != "EVENT" {
        return None;
    }
    let ev: Event = serde_json::from_value(arr.get(2)?.clone()).ok()?;
    let has_topic = ev.tags.iter().any(|t| t.len() >= 2 && t[0] == "t" && t[1] == topic);
    let fresh = ev.created_at + 120 >= now_secs();
    (ev.kind == KIND && has_topic && fresh && verify(&ev)).then_some(ev)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify() {
        let k = Keys::random();
        let ev = k.sign(KIND, vec![vec!["t".into(), "x".into()]], "hi".into(), 1_700_000_000);
        assert!(verify(&ev));
        let mut bad = ev.clone();
        bad.content = "ho".into();
        assert!(!verify(&bad));
    }

    #[test]
    fn parse_filters_topic_and_kind() {
        let k = Keys::random();
        let t = topic_for("482913007");
        let ev = k.sign(KIND, vec![vec!["t".into(), t.clone()]], "c".into(), now_secs());
        let msg = json!(["EVENT", "everywhere", ev]).to_string();
        assert!(parse_event(&msg, &t).is_some());
        assert!(parse_event(&msg, "other").is_none());
        let old = k.sign(KIND, vec![vec!["t".into(), t.clone()]], "c".into(), 1_000);
        assert!(parse_event(&json!(["EVENT", "s", old]).to_string(), &t).is_none(), "stale");
    }

    #[test]
    fn matches_shared_vectors() {
        let v: serde_json::Value = serde_json::from_str(include_str!("../../web/test/vectors.json")).unwrap();
        let s = |k: &str| v[k].as_str().unwrap().to_string();
        assert_eq!(topic_for("482913007"), s("nostr_topic"));
        let sk: [u8; 32] = hex::decode(s("nostr_sk")).unwrap().try_into().unwrap();
        let k = Keys::from_secret(&sk).unwrap();
        assert_eq!(k.pubkey, s("nostr_pubkey"));
        let ev = k.sign(KIND, vec![vec!["t".into(), s("nostr_topic")]], s("nostr_content"), v["nostr_created_at"].as_u64().unwrap());
        assert_eq!(ev.id, s("nostr_id"));
        // The JS side's signature (aux randomness differs) must verify here too.
        let js = Event { sig: s("nostr_sig_js"), ..ev };
        assert!(verify(&js), "JS signature verifies in Rust");
    }

    #[test]
    #[ignore]
    fn print_vectors() {
        let sk: [u8; 32] = core::array::from_fn(|i| (i + 1) as u8);
        let k = Keys::from_secret(&sk).unwrap();
        let topic = topic_for("482913007");
        let content = r#"{"t":"hello","sid":"AAEC"}"#;
        let ev = k.sign(KIND, vec![vec!["t".into(), topic.clone()]], content.into(), 1_700_000_000);
        println!(
            "{}",
            json!({"nostr_sk": hex::encode(sk), "nostr_pubkey": ev.pubkey, "nostr_topic": topic,
                   "nostr_content": content, "nostr_created_at": 1_700_000_000u64, "nostr_id": ev.id, "nostr_sig": ev.sig})
        );
    }
}
