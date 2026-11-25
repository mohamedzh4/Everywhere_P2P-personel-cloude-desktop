// Connecting to the PC: meeting point (Nostr relays) + password handshake +
// WebRTC. The host side is host/src/signaling.rs; message shapes are there.

import * as pake from './pake.js';
import { DEFAULT_RELAYS, KIND, RelayPool, makeEvent, topicFor } from './nostr.js';

// Three servers: if this network gives a different public port to each, the
// PC sees it in the offer and also tries the ports that come next.
const STUN = [{ urls: ['stun:stun.l.google.com:19302', 'stun:stun1.l.google.com:19302', 'stun:stun.cloudflare.com:3478'] }];
const { b64 } = pake;
const enc = new TextEncoder();
const dec = new TextDecoder();

/** What this browser's network looks like, from its own ICE candidates. */
export function describeNetwork(sdp) {
  const ports = new Map();
  for (const line of (sdp || '').split(/\r?\n/)) {
    const f = line.replace(/^a=candidate:/, '').split(' ');
    if (!line.startsWith('a=candidate:') || f[6] !== 'typ' || f[7] !== 'srflx' || f[4].includes(':')) continue;
    if (!ports.has(f[4])) ports.set(f[4], new Set());
    ports.get(f[4]).add(Number(f[5]));
  }
  if (!ports.size) return 'This network did not let the browser find its public address (UDP may be blocked here).';
  const p = [...ports.values()][0];
  if (p.size < 2) return 'This network looks easy to reach.';
  return 'This network gives a new public port for every destination (a strict network, common on phone hotspots).';
}

export class ConnectError extends Error {
  constructor(kind, message) { super(message); this.kind = kind; }
}

function waitFor(pool, sid, predicate, ms) {
  return new Promise((resolve, reject) => {
    const t = setTimeout(() => { pool.listeners.delete(fn); reject(new ConnectError('timeout', 'timeout')); }, ms);
    const fn = (ev, msg) => {
      if (msg.sid !== sid) return;
      if (msg.t === 'error') {
        clearTimeout(t); pool.listeners.delete(fn);
        reject(new ConnectError(msg.reason, msg.reason === 'wait' ? String(msg.wait) : msg.reason));
      } else if (predicate(ev, msg)) {
        clearTimeout(t); pool.listeners.delete(fn); resolve({ ev, msg });
      }
    };
    pool.listeners.add(fn);
  });
}

/**
 * Connect to a PC. `onStep(text)` reports progress. Resolves with
 * { pc, fast, reliable } once the host has answered; the caller waits for
 * the media connection itself.
 */
export async function connect(pcCode, password, onStep) {
  const code = pcCode.replace(/\D/g, '');
  if (code.length !== 9) throw new ConnectError('code', 'The PC code has 9 digits.');
  const topic = topicFor(code);
  const sk = pake.randomBytes(32);
  const myPub = makeEvent(sk, KIND, [], '').pubkey;
  let pool;
  pool = new RelayPool(DEFAULT_RELAYS, topic, (ev) => {
    if (ev.pubkey === myPub) return;
    let msg;
    try { msg = JSON.parse(ev.content); } catch (_) { return; }
    if (msg?.v !== 1) return;
    for (const l of [...pool.listeners]) l(ev, msg);
  });
  pool.listeners = new Set();
  const send = (body, to) => pool.publish(makeEvent(sk, KIND, [['t', topic], ...(to ? [['p', to]] : [])], JSON.stringify(body)));

  try {
    onStep('Reaching the meeting point…');
    if ((await pool.ready(1, 6000)) === 0) {
      throw new ConnectError('relays', 'Cannot reach the meeting point. Check the internet connection; some networks block it.');
    }

    // WebRTC offer first, so the handshake round trips stay short.
    const pc = new RTCPeerConnection({ iceServers: STUN, bundlePolicy: 'max-bundle' });
    const video = pc.addTransceiver('video', { direction: 'recvonly' });
    pc.addTransceiver('audio', { direction: 'recvonly' });
    const caps = RTCRtpReceiver.getCapabilities?.('video');
    if (caps && video.setCodecPreferences) {
      try { video.setCodecPreferences(caps.codecs.filter((c) => /h264|rtx|red|ulpfec/i.test(c.mimeType))); } catch (_) {}
    }
    const fast = pc.createDataChannel('fast', { ordered: false, maxRetransmits: 0 });
    const reliable = pc.createDataChannel('reliable');
    fast.binaryType = reliable.binaryType = 'arraybuffer';
    await pc.setLocalDescription(await pc.createOffer());
    await new Promise((res) => {
      if (pc.iceGatheringState === 'complete') return res();
      const t = setTimeout(res, 3000);
      pc.addEventListener('icegatheringstatechange', () => {
        if (pc.iceGatheringState === 'complete') { clearTimeout(t); res(); }
      });
    });

    onStep('Looking for your PC…');
    const sid = pake.randomBytes(pake.SID_LEN);
    const sidB64 = b64.enc(sid);
    const { ya, yaBytes } = pake.initiate(code, password, sid);
    const ackP = waitFor(pool, sidB64, (_ev, m) => m.t === 'ack', 12000);
    send({ v: 1, t: 'hello', sid: sidB64, ya: b64.enc(yaBytes) });
    let ack;
    try { ack = await ackP; } catch (e) {
      if (e.kind === 'timeout') throw new ConnectError('offline', 'Your PC did not answer. Is Everywhere running on it, and is the PC code right?');
      if (e.kind === 'paused') throw new ConnectError('paused', 'Sharing is turned off on your PC. Click Start sharing in the Everywhere window on it.');
      if (e.kind === 'locked') throw new ConnectError('locked', 'Your PC locked itself after too many wrong passwords. Unlock it from the Everywhere status page on the PC.');
      if (e.kind === 'wait') throw new ConnectError('wait', `Too many tries. Wait ${e.message} seconds and try again.`);
      throw e;
    }

    onStep('Checking the password…');
    const keys = pake.finish(sid, ya, yaBytes, b64.dec(ack.msg.yb));
    if (!keys || !pake.tagsEqual(keys.tagH, b64.dec(ack.msg.tag))) {
      throw new ConnectError('password', 'Wrong password for this PC code.');
    }
    const host = ack.ev.pubkey;

    onStep('Connecting to your PC…');
    const sealed = await pake.seal(keys.kB2h, pake.aad('b2h', sid), enc.encode(pc.localDescription.sdp));
    const answerP = waitFor(pool, sidB64, (ev, m) => m.t === 'answer' && ev.pubkey === host, 15000);
    send({ v: 1, t: 'offer', sid: sidB64, tag: b64.enc(keys.tagB), sdp: b64.enc(sealed) }, host);
    let answer;
    try { answer = await answerP; } catch (e) {
      if (e.kind === 'failed') throw new ConnectError('failed', 'Your PC could not start the stream. Look at its status page.');
      throw new ConnectError('offline', 'Your PC stopped answering.');
    }
    const sdp = await pake.open(keys.kH2b, pake.aad('h2b', sid), b64.dec(answer.msg.sdp));
    if (!sdp) throw new ConnectError('tamper', 'The answer was damaged on the way. Try again.');
    await pc.setRemoteDescription({ type: 'answer', sdp: dec.decode(sdp) });
    return { pc, fast, reliable };
  } finally {
    // The relays are only for the hello; media goes straight to the PC.
    setTimeout(() => pool.close(), 2000);
  }
}
