// Web client tests. Run from web/:  node test/run.mjs
import { register } from 'node:module';
import { readFileSync } from 'node:fs';

register('./loader.mjs', import.meta.url);

let failed = 0, passed = 0;
async function test(name, fn) {
  try { await fn(); passed++; console.log(`ok   ${name}`); }
  catch (e) { failed++; console.log(`FAIL ${name}\n     ${e.stack || e}`); }
}
const eq = (a, b, msg) => { if (a !== b) throw new Error(`${msg}: ${a} !== ${b}`); };

const P = await import('../js/pake.js');
const N = await import('../js/nostr.js');
const I = await import('../js/input.js');
const { hex } = P;
const v = JSON.parse(readFileSync(new URL('./vectors.json', import.meta.url)));

await test('pake: matches Rust vectors byte for byte', async () => {
  const sid = hex.dec(v.sid);
  const { ya, yaBytes } = P.initiate(v.pc_code, v.password, sid, hex.dec(v.ya_wide));
  eq(hex.enc(yaBytes), v.ya, 'Ya');
  const r = P.respond(v.pc_code, v.password, sid, yaBytes, hex.dec(v.yb_wide));
  eq(hex.enc(r.ybBytes), v.yb, 'Yb');
  const k = P.finish(sid, ya, yaBytes, r.ybBytes);
  for (const keys of [k, r.keys]) {
    eq(hex.enc(keys.tagH), v.tag_h, 'tag_h');
    eq(hex.enc(keys.tagB), v.tag_b, 'tag_b');
    eq(hex.enc(keys.kB2h), v.k_b2h, 'k_b2h');
    eq(hex.enc(keys.kH2b), v.k_h2b, 'k_h2b');
  }
  const sealed = await P.seal(k.kB2h, P.aad('b2h', sid), new TextEncoder().encode(v.seal_plain), hex.dec(v.seal_nonce));
  eq(hex.enc(sealed), v.seal_b2h, 'sealed');
  const opened = await P.open(r.keys.kB2h, P.aad('b2h', sid), sealed);
  eq(new TextDecoder().decode(opened), v.seal_plain, 'opened');
});

await test('pake: wrong password → different tags, no decrypt', async () => {
  const sid = P.randomBytes(16);
  const { ya, yaBytes } = P.initiate('482913007', 'right', sid);
  const r = P.respond('482913007', 'wrong', sid, yaBytes, P.randomBytes(64));
  const k = P.finish(sid, ya, yaBytes, r.ybBytes);
  eq(P.tagsEqual(k.tagH, r.keys.tagH), false, 'tags differ');
  const sealed = await P.seal(k.kB2h, P.aad('b2h', sid), new Uint8Array([1, 2, 3]));
  eq(await P.open(r.keys.kB2h, P.aad('b2h', sid), sealed), null, 'cannot open');
});

await test('pake: rejects identity and garbage points', () => {
  eq(P.finish(new Uint8Array(16), 5n, new Uint8Array(32), new Uint8Array(32)), null, 'identity');
  eq(P.finish(new Uint8Array(16), 5n, new Uint8Array(32), new Uint8Array(32).fill(255)), null, 'garbage');
});

await test('nostr: event id and signature verify (BIP-340)', async () => {
  const sk = P.randomBytes(32);
  const ev = N.makeEvent(sk, 25050, [['t', 'abc']], 'hello', 1700000000);
  eq(ev.id.length, 64, 'id length');
  eq(N.verifyEvent(ev), true, 'verifies');
  eq(N.verifyEvent({ ...ev, content: 'tampered' }), false, 'tamper detected');
});

await test('nostr: matches Rust event vector', async () => {
  const sk = hex.dec(v.nostr_sk);
  const ev = N.makeEvent(sk, 25050, [['t', v.nostr_topic]], v.nostr_content, v.nostr_created_at);
  eq(ev.pubkey, v.nostr_pubkey, 'pubkey');
  eq(ev.id, v.nostr_id, 'id');
  eq(N.verifyEvent({ ...ev, sig: v.nostr_sig }), true, 'Rust signature verifies in JS');
});

await test('input: messages encode to the agreed byte layout', () => {
  eq(hex.enc(I.encodeMove(-3, 300)), '01fdff2c01', 'move');
  eq(hex.enc(I.encodeKey(true, 'KeyW')), '0a01044b657957', 'key');
  eq(hex.enc(I.encodeButton(false, 2)), '0b0002', 'button');
  eq(hex.enc(I.encodeWheel(0, -120)), '03000088ff', 'wheel');
});

await test('topic: derived from the PC code', () => {
  eq(N.topicFor('482913007'), v.nostr_topic, 'topic');
});

await test('network: strict networks are recognised from the offer', async () => {
  const { describeNetwork } = await import('../js/connect.js');
  const c = (port, ip = '198.51.100.7') => `a=candidate:1 1 udp 1686052607 ${ip} ${port} typ srflx raddr 0.0.0.0 rport 0`;
  eq(/strict/.test(describeNetwork([c(40001), c(40002)].join('\r\n'))), true, 'two ports = strict');
  eq(/easy/.test(describeNetwork([c(40001), c(40001)].join('\r\n'))), true, 'one port = easy');
  eq(/UDP/.test(describeNetwork('a=candidate:1 1 udp 1 abc.local 5 typ host')), true, 'no public address');
  eq(/easy/.test(describeNetwork([c(40001), c(40009, '2001:db8::1')].join('\r\n'))), true, 'IPv6 ignored');
});

console.log(`\n${passed} passed, ${failed} failed`);
process.exit(failed ? 1 : 0);
