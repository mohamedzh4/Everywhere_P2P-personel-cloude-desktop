// Connection numbers for the overlay: path, round trip, bitrate, frame rate,
// decode time, loss. All from the browser's own getStats().

export function startStats(pc, onUpdate) {
  let prev = null;
  const tick = async () => {
    const r = await pc.getStats();
    let inb, pair;
    r.forEach((s) => {
      if (s.type === 'inbound-rtp' && s.kind === 'video') inb = s;
      if (s.type === 'transport' && s.selectedCandidatePairId) pair = r.get(s.selectedCandidatePairId);
    });
    if (!pair) r.forEach((s) => { if (s.type === 'candidate-pair' && s.nominated && s.state === 'succeeded') pair = s; });
    const out = {};
    if (inb) {
      if (prev) {
        const dt = (inb.timestamp - prev.timestamp) / 1000;
        out.mbps = ((inb.bytesReceived - prev.bytesReceived) * 8) / dt / 1e6;
        const d = inb.framesDecoded - prev.framesDecoded;
        out.decodeMs = d ? ((inb.totalDecodeTime - prev.totalDecodeTime) / d) * 1000 : 0;
      }
      out.fps = inb.framesPerSecond || 0;
      out.size = inb.frameWidth ? `${inb.frameWidth}x${inb.frameHeight}` : '';
      out.lost = inb.packetsLost || 0;
      out.hw = !!inb.powerEfficientDecoder;
      prev = inb;
    }
    if (pair) {
      const local = r.get(pair.localCandidateId), remote = r.get(pair.remoteCandidateId);
      out.rttMs = pair.currentRoundTripTime != null ? pair.currentRoundTripTime * 1000 : null;
      out.path = describe(local, remote);
    }
    onUpdate(out);
  };
  const id = setInterval(() => tick().catch(() => {}), 1000);
  return () => clearInterval(id);
}

function describe(local, remote) {
  if (!remote) return '';
  const addr = remote.address || remote.ip || '';
  const v6 = addr.includes(':');
  if (remote.candidateType === 'relay' || local?.candidateType === 'relay') return 'relay';
  if (v6) return 'IPv6 direct';
  if (remote.candidateType === 'host' && /^(10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.)/.test(addr)) return 'same network';
  return 'direct (IPv4)';
}
