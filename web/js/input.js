// Keyboard, mouse and wheel capture, sent to the host over two data channels:
//   "fast"     unordered, no resend: mouse movement (latest wins, batched)
//   "reliable" ordered: keys, buttons, wheel, release-all
// Byte layout (little-endian) is shared with host/src/input.rs:
//   1  move    i16 dx, i16 dy       (relative, pointer locked)
//   2  abs     u16 x, u16 y         (0..65535 across the video)
//   3  wheel   i16 dx, i16 dy
//   10 key     u8 down, u8 len, utf8 KeyboardEvent.code
//   11 button  u8 down, u8 button  (0 left, 1 middle, 2 right, 3 back, 4 forward)
//   12 release all keys and buttons (focus lost)

const enc = new TextEncoder();

export function encodeMove(dx, dy) {
  const b = new DataView(new ArrayBuffer(5));
  b.setUint8(0, 1); b.setInt16(1, dx, true); b.setInt16(3, dy, true);
  return new Uint8Array(b.buffer);
}
export function encodeAbs(x, y) {
  const b = new DataView(new ArrayBuffer(5));
  b.setUint8(0, 2); b.setUint16(1, x, true); b.setUint16(3, y, true);
  return new Uint8Array(b.buffer);
}
export function encodeWheel(dx, dy) {
  const b = new DataView(new ArrayBuffer(5));
  b.setUint8(0, 3); b.setInt16(1, dx, true); b.setInt16(3, dy, true);
  return new Uint8Array(b.buffer);
}
export function encodeKey(down, code) {
  const c = enc.encode(code).slice(0, 255);
  const out = new Uint8Array(3 + c.length);
  out[0] = 10; out[1] = down ? 1 : 0; out[2] = c.length; out.set(c, 3);
  return out;
}
export function encodeButton(down, button) {
  return new Uint8Array([11, down ? 1 : 0, button]);
}
export const RELEASE_ALL = new Uint8Array([12]);

const clamp16 = (v) => Math.max(-32768, Math.min(32767, Math.round(v)));

/**
 * Wire input on `video` to the two channels. Mouse motion is summed and sent
 * once per animation frame-ish tick (every 4 ms), like Moonlight's batching:
 * fewer packets, no queue building up, nothing lost.
 */
export function attachInput(video, fast, reliable) {
  let dx = 0, dy = 0;
  const down = new Set();
  const send = (ch, data) => { if (ch.readyState === 'open') ch.send(data); };
  const locked = () => document.pointerLockElement === video;

  const flush = setInterval(() => {
    if (dx || dy) {
      send(fast, encodeMove(clamp16(dx), clamp16(dy)));
      dx = 0; dy = 0;
    }
  }, 4);

  const onMove = (e) => {
    if (locked()) { dx += e.movementX; dy += e.movementY; return; }
    const r = video.getBoundingClientRect();
    // Map to the visible picture inside the letterboxed element.
    const vw = video.videoWidth || r.width, vh = video.videoHeight || r.height;
    const s = Math.min(r.width / vw, r.height / vh);
    const pw = vw * s, ph = vh * s, ox = r.left + (r.width - pw) / 2, oy = r.top + (r.height - ph) / 2;
    const x = (e.clientX - ox) / pw, y = (e.clientY - oy) / ph;
    if (x < 0 || x > 1 || y < 0 || y > 1) return;
    send(fast, encodeAbs(Math.round(x * 65535), Math.round(y * 65535)));
  };
  const onDown = (e) => {
    if (!locked()) {
      // First click captures the mouse (raw movement for games).
      video.requestPointerLock?.({ unadjustedMovement: true })?.catch?.(() => video.requestPointerLock());
    }
    send(reliable, encodeButton(true, e.button)); e.preventDefault();
  };
  const onUp = (e) => { send(reliable, encodeButton(false, e.button)); e.preventDefault(); };
  const onWheel = (e) => {
    // Browsers report pixels or lines; normalise to Windows' 120 per notch.
    const k = e.deltaMode === 1 ? 40 : e.deltaMode === 2 ? 120 * 3 : 1;
    send(reliable, encodeWheel(clamp16(e.deltaX * k), clamp16(-e.deltaY * k)));
    e.preventDefault();
  };
  const onKey = (e) => {
    if (!locked() && document.fullscreenElement == null) return; // not playing
    const isDown = e.type === 'keydown';
    if (isDown && e.repeat) { e.preventDefault(); return; } // host OS does its own repeat
    if (isDown) down.add(e.code); else down.delete(e.code);
    send(reliable, encodeKey(isDown, e.code));
    e.preventDefault();
  };
  const releaseAll = () => { down.clear(); send(reliable, RELEASE_ALL); };

  video.addEventListener('mousemove', onMove);
  document.addEventListener('mousemove', (e) => { if (locked()) onMove(e); });
  video.addEventListener('mousedown', onDown);
  video.addEventListener('mouseup', onUp);
  video.addEventListener('wheel', onWheel, { passive: false });
  video.addEventListener('contextmenu', (e) => e.preventDefault());
  window.addEventListener('keydown', onKey, true);
  window.addEventListener('keyup', onKey, true);
  window.addEventListener('blur', releaseAll);
  document.addEventListener('visibilitychange', () => { if (document.hidden) releaseAll(); });
  document.addEventListener('pointerlockchange', () => { if (!locked()) releaseAll(); });

  return () => clearInterval(flush);
}

/** Fullscreen + Keyboard Lock, so Esc, Alt+Tab, Win reach the game (Chrome/Edge). */
export async function enterPlayMode(el) {
  await el.requestFullscreen?.({ navigationUI: 'hide' });
  try { await navigator.keyboard?.lock?.(); } catch (_) {}
}
