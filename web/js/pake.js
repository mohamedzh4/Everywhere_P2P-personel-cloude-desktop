// Password handshake, browser side. Mirror of host/src/pake.rs; the protocol
// is described there. test/vectors.json pins both sides to the same bytes.

import { ristretto255, ristretto255_hasher } from '@noble/curves/ed25519.js';
import { bytesToNumberLE } from '@noble/curves/utils.js';
import { sha256, sha512 } from '@noble/hashes/sha2.js';
import { hmac } from '@noble/hashes/hmac.js';

const Point = ristretto255.Point;
const enc = new TextEncoder();

export const SID_LEN = 16;

function concat(...parts) {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let o = 0;
  for (const p of parts) { out.set(p, o); o += p.length; }
  return out;
}

function lp(x) {
  const len = new Uint8Array(4);
  new DataView(len.buffer).setUint32(0, x.length, false);
  return concat(len, x);
}

export function generator(pcCode, password, sid) {
  const m = concat(enc.encode('everywhere/cpace/v1'), lp(enc.encode(pcCode)), lp(enc.encode(password)), sid);
  return ristretto255_hasher.deriveToCurve(sha512(m));
}

export function randomBytes(n) {
  return crypto.getRandomValues(new Uint8Array(n));
}

function scalarFromWide(wide) {
  const s = Point.Fn.create(bytesToNumberLE(wide));
  if (s === 0n) throw new Error('zero scalar');
  return s;
}

function decodePoint(bytes) {
  try {
    const p = Point.fromBytes(bytes);
    return p.is0() ? null : p;
  } catch (_) {
    return null;
  }
}

/** Start: returns the secret scalar and Ya to send. `yaWide` only for tests. */
export function initiate(pcCode, password, sid, yaWide = randomBytes(64)) {
  const ya = scalarFromWide(yaWide);
  const yaBytes = generator(pcCode, password, sid).multiply(ya).toBytes();
  return { ya, yaBytes };
}

export function derive(sid, yaBytes, ybBytes, kBytes) {
  const isk = sha512(concat(enc.encode('everywhere/isk/v1'), sid, yaBytes, ybBytes, kBytes));
  const kdf = (label) => sha256(concat(enc.encode(label), isk));
  return {
    tagH: hmac(sha256, isk, enc.encode('everywhere/confirm/host')),
    tagB: hmac(sha256, isk, enc.encode('everywhere/confirm/browser')),
    kB2h: kdf('everywhere/key/b2h'),
    kH2b: kdf('everywhere/key/h2b'),
  };
}

/** Finish with the host's Yb. Returns keys, or null for a bad point. */
export function finish(sid, ya, yaBytes, ybBytes) {
  const yb = decodePoint(ybBytes);
  if (!yb) return null;
  const k = yb.multiply(ya);
  if (k.is0()) return null;
  return derive(sid, yaBytes, ybBytes, k.toBytes());
}

/** Host side, for tests only (the real host is Rust). */
export function respond(pcCode, password, sid, yaBytes, ybWide) {
  const ya = decodePoint(yaBytes);
  if (!ya) return null;
  const yb = scalarFromWide(ybWide);
  const ybBytes = generator(pcCode, password, sid).multiply(yb).toBytes();
  const k = ya.multiply(yb);
  if (k.is0()) return null;
  return { ybBytes, keys: derive(sid, yaBytes, ybBytes, k.toBytes()) };
}

export function tagsEqual(a, b) {
  if (a.length !== b.length) return false;
  let d = 0;
  for (let i = 0; i < a.length; i++) d |= a[i] ^ b[i];
  return d === 0;
}

export function aad(direction, sid) {
  return concat(enc.encode(`everywhere/${direction}/`), sid);
}

/** AES-256-GCM: 12-byte nonce || ciphertext || tag. */
export async function seal(key, aadBytes, plaintext, nonce = randomBytes(12)) {
  const k = await crypto.subtle.importKey('raw', key, 'AES-GCM', false, ['encrypt']);
  const ct = new Uint8Array(await crypto.subtle.encrypt({ name: 'AES-GCM', iv: nonce, additionalData: aadBytes }, k, plaintext));
  return concat(nonce, ct);
}

export async function open(key, aadBytes, sealed) {
  if (sealed.length < 28) return null;
  const k = await crypto.subtle.importKey('raw', key, 'AES-GCM', false, ['decrypt']);
  try {
    return new Uint8Array(await crypto.subtle.decrypt(
      { name: 'AES-GCM', iv: sealed.subarray(0, 12), additionalData: aadBytes }, k, sealed.subarray(12)));
  } catch (_) {
    return null;
  }
}

export const hex = {
  enc: (b) => Array.from(b, (x) => x.toString(16).padStart(2, '0')).join(''),
  dec: (s) => Uint8Array.from(s.match(/../g) || [], (h) => parseInt(h, 16)),
};
export const b64 = {
  enc: (b) => btoa(String.fromCharCode(...b)),
  dec: (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0)),
};
