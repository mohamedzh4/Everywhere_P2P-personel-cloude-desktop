// Everywhere web client, phase 1 (LAN test).
// Connects with WebRTC, plays the video with the lowest-latency settings the
// browser offers, and measures capture-to-screen latency from the stamp the
// host draws at the top of every frame (see host/src/stamp.rs).

const token = new URLSearchParams(location.search).get('t') || '';
const $ = (id) => document.getElementById(id);
const video = $('video');

// Live numbers, also exposed for automated tests.
const S = (window.everywhere = {
  state: 'idle', latency: [], toReceive: [], inBrowser: [], fps: 0, bitrate: 0, decodeMs: 0, jitterMs: 0,
  dropped: 0, lost: 0, nack: 0, pli: 0, rtt: null, path: '', decoder: '', size: '',
});

// ---------- latency stamp reader (mirror of host/src/stamp.rs) ----------
const BLOCKS = 64, DATA_BITS = 48;
const canvas = document.createElement('canvas');
const ctx = canvas.getContext('2d', { willReadFrequently: true });

function readStamp() {
  const w = video.videoWidth;
  if (!w) return null;
  const b = Math.floor(w / BLOCKS) & ~1;
  const used = 2 + DATA_BITS + 8;
  canvas.width = used * b; canvas.height = b;
  ctx.drawImage(video, 0, 0, used * b, b, 0, 0, used * b, b);
  const px = ctx.getImageData(0, 0, used * b, b).data;
  const bit = (i) => {
    const x = i * b + (b >> 1), y = b >> 1, o = (y * used * b + x) * 4;
    return (px[o] + px[o + 1] + px[o + 2]) / 3 > 128;
  };
  if (!bit(0) || bit(1)) return null;
  let v = 0n;
  for (let i = 0; i < DATA_BITS; i++) v = (v << 1n) | (bit(2 + i) ? 1n : 0n);
  let check = 0;
  for (let i = 0; i < 8; i++) check = (check << 1) | (bit(2 + DATA_BITS + i) ? 1 : 0);
  let want = 0;
  for (let i = 0; i < 6; i++) want ^= Number((v >> BigInt(8 * (5 - i))) & 0xffn);
  return check === want ? Number(v) / 10 : null; // ms since Unix epoch
}

function onFrame(_now, meta) {
  const stampMs = readStamp();
  if (stampMs !== null) {
    // expectedDisplayTime: when the compositor will show this frame.
    const shown = performance.timeOrigin + meta.expectedDisplayTime;
    S.latency.push(shown - stampMs);
    // Split: host + network (until the last packet arrived) vs browser
    // (jitter buffer + decode + compositor).
    if (meta.receiveTime) {
      const recv = performance.timeOrigin + meta.receiveTime;
      S.toReceive.push(recv - stampMs);
      S.inBrowser.push(shown - recv);
    }
    for (const a of [S.latency, S.toReceive, S.inBrowser]) if (a.length > 600) a.shift();
  }
  video.requestVideoFrameCallback(onFrame);
}

// ---------- stats ----------
const pct = (a, p) => { if (!a.length) return NaN; const s = [...a].sort((x, y) => x - y); return s[Math.min(s.length - 1, Math.floor(p * s.length))]; };
let prev = null;

async function pollStats(pc) {
  const r = await pc.getStats();
  let inb, pair, local, remote;
  r.forEach((s) => {
    if (s.type === 'inbound-rtp' && s.kind === 'video') inb = s;
    if (s.type === 'transport' && s.selectedCandidatePairId) pair = r.get(s.selectedCandidatePairId);
  });
  if (!pair) r.forEach((s) => { if (s.type === 'candidate-pair' && s.nominated && s.state === 'succeeded') pair = s; });
  if (pair) { local = r.get(pair.localCandidateId); remote = r.get(pair.remoteCandidateId); }
  if (inb) {
    if (prev) {
      const dt = (inb.timestamp - prev.timestamp) / 1000;
      S.bitrate = ((inb.bytesReceived - prev.bytesReceived) * 8) / dt / 1e6;
      const dDec = inb.framesDecoded - prev.framesDecoded;
      S.decodeMs = dDec ? ((inb.totalDecodeTime - prev.totalDecodeTime) / dDec) * 1000 : 0;
      const dEm = inb.jitterBufferEmittedCount - prev.jitterBufferEmittedCount;
      S.jitterMs = dEm ? ((inb.jitterBufferDelay - prev.jitterBufferDelay) / dEm) * 1000 : 0;
    }
    S.fps = inb.framesPerSecond || 0;
    S.dropped = inb.framesDropped || 0;
    S.lost = inb.packetsLost || 0;
    S.nack = inb.nackCount || 0;
    S.pli = inb.pliCount || 0;
    S.decoder = `${inb.decoderImplementation || '?'}${inb.powerEfficientDecoder ? ' (hw)' : ''}`;
    S.size = `${inb.frameWidth}x${inb.frameHeight}`;
    prev = inb;
  }
  if (pair) {
    S.rtt = pair.currentRoundTripTime != null ? pair.currentRoundTripTime * 1000 : null;
    S.path = `${local?.candidateType || '?'} → ${remote?.candidateType || '?'} ${remote?.protocol || ''}`;
  }
  const l = S.latency;
  $('stats').textContent =
    `latency  p50 ${pct(l, 0.5).toFixed(1)} ms  p95 ${pct(l, 0.95).toFixed(1)} ms  (capture → screen)\n` +
    `  split  host+network ${pct(S.toReceive, 0.5).toFixed(1)} ms  browser ${pct(S.inBrowser, 0.5).toFixed(1)} ms\n` +
    `video    ${S.size} @ ${S.fps.toFixed(0)} fps  ${S.bitrate.toFixed(1)} Mbit/s  ${S.decoder}\n` +
    `browser  decode ${S.decodeMs.toFixed(1)} ms  jitter buffer ${S.jitterMs.toFixed(1)} ms\n` +
    `network  rtt ${S.rtt == null ? '?' : S.rtt.toFixed(1)} ms  lost ${S.lost}  nack ${S.nack}  pli ${S.pli}  dropped ${S.dropped}\n` +
    `path     ${S.path}`;
}

// ---------- connect ----------
async function connect() {
  $('go').disabled = true;
  $('status').textContent = 'Connecting…';
  S.state = 'connecting';
  const pc = new RTCPeerConnection({ bundlePolicy: 'max-bundle' });
  const tr = pc.addTransceiver('video', { direction: 'recvonly' });
  // H.264 only for now (hardware decode everywhere).
  const caps = RTCRtpReceiver.getCapabilities('video');
  if (caps && tr.setCodecPreferences) {
    const h264 = caps.codecs.filter((c) => /h264|rtx|red|ulpfec/i.test(c.mimeType));
    try { tr.setCodecPreferences(h264); } catch (_) { /* older browsers */ }
  }
  pc.ontrack = (e) => {
    video.srcObject = e.streams[0] || new MediaStream([e.track]);
    try { e.receiver.jitterBufferTarget = 0; } catch (_) {}
    video.requestVideoFrameCallback(onFrame);
  };
  pc.onconnectionstatechange = () => {
    S.state = pc.connectionState;
    if (pc.connectionState === 'connected') {
      $('start').hidden = true; $('stats').hidden = false;
    } else if (['failed', 'disconnected', 'closed'].includes(pc.connectionState)) {
      $('start').hidden = false; $('stats').hidden = true; $('go').disabled = false;
      $('status').innerHTML = `<span class="bad">Connection ${pc.connectionState}.</span>`;
    }
  };

  await pc.setLocalDescription(await pc.createOffer());
  // No trickle in phase 1: wait for local candidates (max 2 s).
  await new Promise((res) => {
    if (pc.iceGatheringState === 'complete') return res();
    const t = setTimeout(res, 2000);
    pc.addEventListener('icegatheringstatechange', () => { if (pc.iceGatheringState === 'complete') { clearTimeout(t); res(); } });
  });
  const resp = await fetch(`/offer?t=${encodeURIComponent(token)}`, { method: 'POST', body: pc.localDescription.sdp });
  if (!resp.ok) {
    $('status').innerHTML = `<span class="bad">Host refused: ${await resp.text()}</span>`;
    $('go').disabled = false; S.state = 'refused';
    return;
  }
  await pc.setRemoteDescription({ type: 'answer', sdp: await resp.text() });
  setInterval(() => pollStats(pc).catch(() => {}), 1000);
  window.everywhere.pc = pc;
}

$('go').addEventListener('click', () => connect().catch((e) => {
  $('status').innerHTML = `<span class="bad">${e}</span>`; $('go').disabled = false; S.state = 'error';
}));
// Double-click: fullscreen.
video.addEventListener('dblclick', () => document.documentElement.requestFullscreen?.());
