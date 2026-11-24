// Everywhere web client: the page people open on any laptop.

import { connect } from './connect.js';
import { attachInput, enterPlayMode } from './input.js';
import { startStats } from './stats.js';

const $ = (id) => document.getElementById(id);
const REMEMBER_KEY = 'everywhere.code';

// ---------- connect screen ----------
const store = {
  get: () => { try { return localStorage.getItem(REMEMBER_KEY) || ''; } catch (_) { return ''; } },
  set: (v) => { try { v ? localStorage.setItem(REMEMBER_KEY, v) : localStorage.removeItem(REMEMBER_KEY); } catch (_) {} },
};

// Show the PC code as 482-913-007 while typing.
$('code').addEventListener('input', (e) => {
  const d = e.target.value.replace(/\D/g, '').slice(0, 9);
  e.target.value = d.replace(/(\d{3})(?=\d)/g, '$1-');
});
// Code may come from a link: …/#482913007 (never the password).
const fromHash = location.hash.replace(/\D/g, '').slice(0, 9);
const remembered = store.get();
$('code').value = (fromHash || remembered).replace(/(\d{3})(?=\d)/g, '$1-');
$('remember').checked = !!remembered;
(($('code').value.length === 11) ? $('password') : $('code')).focus();

if (!window.RTCPeerConnection || !window.isSecureContext) {
  $('compat').hidden = false;
  $('compat').textContent = 'This browser cannot do this. Use a recent Chrome, Edge or Firefox.';
}

function step(text, bad = false) {
  $('step').textContent = text;
  $('step').classList.toggle('bad', bad);
}

$('form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const code = $('code').value, password = $('password').value.trim();
  if (!password) return step('Type the password shown on your PC.', true);
  store.set($('remember').checked ? code.replace(/\D/g, '') : '');
  $('go').disabled = true;
  try {
    const session = await connect(code, password, (t) => step(t));
    $('password').value = '';
    await play(session);
  } catch (err) {
    step(err.message || String(err), true);
  } finally {
    $('go').disabled = false;
  }
});

// ---------- player ----------
async function play({ pc, fast, reliable }) {
  const video = $('video');
  const stream = new MediaStream();
  pc.ontrack = (e) => {
    stream.addTrack(e.track);
    // Ask the browser to show frames as soon as they are decoded.
    try { e.receiver.jitterBufferTarget = 0; } catch (_) {}
  };
  video.srcObject = stream;

  await new Promise((resolve, reject) => {
    const t = setTimeout(() => reject(new Error(
      'Your PC answered, but this network blocks a direct connection. Try a phone hotspot, or another network.')), 20000);
    pc.addEventListener('connectionstatechange', () => {
      if (pc.connectionState === 'connected') { clearTimeout(t); resolve(); }
      if (pc.connectionState === 'failed') { clearTimeout(t); reject(new Error(
        'Your PC answered, but no direct path works between the two networks. Try a phone hotspot.')); }
    });
  });

  $('connect').hidden = true;
  $('player').hidden = false;
  step('');
  video.muted = false; // the Connect click allows sound
  video.play().catch(() => { video.muted = true; video.play(); });

  const stopInput = attachInput(video, fast, reliable);
  let last = {};
  const stopStats = startStats(pc, (s) => {
    last = s;
    $('path').textContent = s.path || '…';
    $('ping').textContent = s.rttMs != null ? `${Math.round(s.rttMs)} ms` : '';
    $('stats').textContent =
      `${s.size || ''} · ${(s.fps || 0).toFixed(0)} fps · ${(s.mbps || 0).toFixed(1)} Mbit/s\n` +
      `decode ${(s.decodeMs || 0).toFixed(1)} ms${s.hw ? ' (hardware)' : ''} · lost ${s.lost ?? 0}\n` +
      `path ${s.path || '?'} · round trip ${s.rttMs != null ? s.rttMs.toFixed(0) : '?'} ms`;
  });

  // Bar hides while playing; shows on mouse move when the mouse is free.
  let idle;
  const wake = () => {
    $('bar').classList.remove('idle');
    clearTimeout(idle);
    idle = setTimeout(() => $('bar').classList.add('idle'), 2500);
  };
  document.addEventListener('mousemove', () => { if (!document.pointerLockElement) wake(); });
  wake();
  document.addEventListener('pointerlockchange', () => {
    $('hint').textContent = document.pointerLockElement
      ? 'Hold Esc to free the mouse'
      : 'Click the picture to control your PC';
  });

  $('fullscreen').onclick = () => enterPlayMode(document.documentElement);
  $('toggle-stats').onclick = () => { $('stats').hidden = !$('stats').hidden; };
  const end = (msg) => {
    stopInput(); stopStats();
    try { pc.close(); } catch (_) {}
    video.srcObject = null;
    if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
    $('player').hidden = true;
    $('connect').hidden = false;
    step(msg || 'Disconnected.', !!msg);
  };
  $('disconnect').onclick = () => end();
  pc.addEventListener('connectionstatechange', () => {
    if (['failed', 'closed', 'disconnected'].includes(pc.connectionState)) {
      end(last.path ? 'The connection to your PC was lost.' : 'Could not keep the connection.');
    }
  });
}
