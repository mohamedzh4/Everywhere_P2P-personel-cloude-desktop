// Meeting point over public Nostr relays, browser side. Mirror of
// host/src/nostr.rs (kind, topic, relay list).

import { schnorr } from '@noble/curves/secp256k1.js';
import { sha256 } from '@noble/hashes/sha2.js';

export const KIND = 25050;
export const DEFAULT_RELAYS = [
  'wss://relay.damus.io',
  'wss://nos.lol',
  'wss://relay.primal.net',
  'wss://nostr.mom',
  'wss://relay.nostr.band',
];

const enc = new TextEncoder();
const toHex = (b) => Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');
const fromHex = (s) => Uint8Array.from(s.match(/../g) || [], (h) => parseInt(h, 16));

export function topicFor(pcCode) {
  return toHex(sha256(enc.encode(`everywhere/v1/${pcCode}`)));
}

function eventId(pubkey, createdAt, kind, tags, content) {
  return sha256(enc.encode(JSON.stringify([0, pubkey, createdAt, kind, tags, content])));
}

export function makeEvent(sk, kind, tags, content, createdAt = Math.floor(Date.now() / 1000)) {
  const pubkey = toHex(schnorr.getPublicKey(sk));
  const id = eventId(pubkey, createdAt, kind, tags, content);
  const sig = toHex(schnorr.sign(id, sk));
  return { id: toHex(id), pubkey, created_at: createdAt, kind, tags, content, sig };
}

export function verifyEvent(ev) {
  try {
    const id = eventId(ev.pubkey, ev.created_at, ev.kind, ev.tags, ev.content);
    if (toHex(id) !== ev.id) return false;
    return schnorr.verify(fromHex(ev.sig), id, fromHex(ev.pubkey));
  } catch (_) {
    return false;
  }
}

/**
 * Connections to several relays. Publishes to all, de-duplicates incoming
 * events, reconnects with back-off.
 */
export class RelayPool {
  constructor(relays, topic, onEvent) {
    this.topic = topic;
    this.onEvent = onEvent;
    this.seen = new Set();
    this.sockets = new Map();
    this.closed = false;
    this.pending = [];
    for (const url of relays) this.#connect(url, 1000);
  }

  get connected() {
    return [...this.sockets.values()].filter((ws) => ws.readyState === WebSocket.OPEN).length;
  }

  #connect(url, backoff) {
    if (this.closed) return;
    let ws;
    try { ws = new WebSocket(url); } catch (_) { return; }
    this.sockets.set(url, ws);
    ws.onopen = () => {
      ws.send(JSON.stringify(['REQ', 'everywhere',
        { kinds: [KIND], '#t': [this.topic], since: Math.floor(Date.now() / 1000) - 10 }]));
      for (const m of this.pending) ws.send(m);
    };
    ws.onmessage = (m) => {
      let msg;
      try { msg = JSON.parse(m.data); } catch (_) { return; }
      if (!Array.isArray(msg) || msg[0] !== 'EVENT' || !msg[2]) return;
      const ev = msg[2];
      if (this.seen.has(ev.id)) return;
      if (ev.kind !== KIND || !ev.tags?.some((t) => t[0] === 't' && t[1] === this.topic)) return;
      if (ev.created_at + 120 < Date.now() / 1000 || !verifyEvent(ev)) return;
      this.seen.add(ev.id);
      this.onEvent(ev);
    };
    ws.onclose = () => {
      if (!this.closed) setTimeout(() => this.#connect(url, Math.min(backoff * 2, 60000)), backoff);
    };
    ws.onerror = () => {};
  }

  publish(ev) {
    const m = JSON.stringify(['EVENT', ev]);
    // Kept for relays that connect later in this session.
    this.pending.push(m);
    if (this.pending.length > 8) this.pending.shift();
    for (const ws of this.sockets.values()) if (ws.readyState === WebSocket.OPEN) ws.send(m);
  }

  /** Resolves once at least `n` relays are open, or after `ms`. */
  ready(n = 1, ms = 4000) {
    return new Promise((res) => {
      const t0 = Date.now();
      const tick = () => (this.connected >= n || Date.now() - t0 > ms ? res(this.connected) : setTimeout(tick, 50));
      tick();
    });
  }

  close() {
    this.closed = true;
    for (const ws of this.sockets.values()) try { ws.close(); } catch (_) {}
  }
}
