# 2026-11-24: phase 1, first stream (one machine)

Host and browser on the same Mac (Apple Silicon, macOS 26.5.1). Browser: the
Claude app's built-in Chromium. Test pattern 1280x720 @ 60 fps, OpenH264
(software), 8 Mbit/s target, WebRTC over loopback (`--bind 127.0.0.1`).

## Works

- WebRTC connects (ICE: browser prflx → host udp), H.264 plays, 0 packets
  lost, 0 keyframe requests.
- Latency stamp read back from the decoded video on every frame.
- `cargo test`: 3/3 (stamp round trip in I420 and RGBA, corrupt stamp rejected).

## Measured

| Run | Latency p50 / p95 (capture → screen) | Host + network p50 | Browser p50 | fps | Encode | Decode | Jitter buffer |
|---|---|---|---|---|---|---|---|
| 1 (start screen still covering the video) | 59.3 / 59.9 ms | not split yet | | 60 | 8.8 ms | 1.4 ms | 8.7 ms |
| 2 (video visible) | 74.7 / 93.8 ms | 22.2 ms | 50.8 ms | 40 | | 1.8 ms | 15.0 ms |

**Far from the ≤ 38 ms target. Not usable for games yet.** Where the time goes:

1. **Browser ~50 ms** after the last packet arrives. The jitter buffer holds
   9–15 ms even with `jitterBufferTarget = 0`, so the playout-delay extension
   is probably not negotiated. The rest is the compositor. Run 2 also dropped
   to 40 fps, likely because host, encoder and browser share one CPU.
2. **Host + network ~22 ms** on loopback: about 9 ms of software encode, and
   the remainder is probably str0m's pacer. To check.

## Failed

- **The LAN address never connected; ICE stayed at
  "checking".** Loopback works, so the likely cause is macOS Local Network
  privacy blocking the app's processes. Not yet confirmed.

## Next

1. Check whether playout-delay is in the SDP and add it to the extension map.
   Log str0m's pacer queue time.
2. Run in real Chrome, and with host and browser on two machines.
3. Phase 1b on the Windows gaming PC: DXGI capture and NVENC, which should take
   encode from 9 ms to about 3 ms (Estimate).

## Follow-up the same day: playout delay and pacing

**Playout delay: fixed.** Chrome offered the playout-delay extension (id 5),
but str0m's answer dropped it because it isn't in str0m's default extension map.
`RtcConfig::set_extension(5, Extension::PlayoutDelay)` adds it. Now the answer
carries it and every frame says "min 0, max 0".

**Pacing: checked, not the problem.** The host now measures how long each frame
waits inside str0m, from `write()` until its last RTP packet goes to the socket.
SRTP leaves the RTP header in clear, so the timestamp and payload type can be
read. A `--no-pacing` switch (no bandwidth estimate, so str0m uses its null
pacer) was added for later A/B tests; it was not needed here.

**Software encoder:** under load it slowed to 19–31 ms per frame. `num_threads(4)`
brought it back to 13–17 ms.

| Run | Latency p50 / p95 | Host + network p50 | Browser p50 | Jitter buffer | Send queue p50 / p95 | Encode | fps |
|---|---|---|---|---|---|---|---|
| 2 (before) | 74.7 / 93.8 ms | 22.2 ms | 50.8 ms | 15.0 ms | not measured | not logged | 40 |
| 3 (+ playout delay) | 51.6 / 70.0 ms | 25.9 ms | 27.2 ms | 0.25 ms | 0.23 / 0.52 ms | 19–31 ms | 31 |
| 4 (+ 4 encoder threads) | **41.3 / 50.4 ms** | 13.3 ms | 27.8 ms | 0.2 ms | 0.13 / 0.21 ms | 13–17 ms | 46 |

Where the remaining ~41 ms goes:

- **Host ~13 ms**: almost all of it is OpenH264 software encode on a busy CPU.
  NVENC in phase 1b should take this to about 3–5 ms (Estimate).
- **Browser ~28 ms**: decode 1.8 ms, jitter buffer 0.2 ms, and about 26 ms of
  compositor and vsync wait until the frame is shown. This was measured in the
  Claude app's built-in browser pane; real Chrome, fullscreen, may differ. To
  measure next.
- Frame rate still dips below 60 (46 fps) because the host and browser share
  this Mac's CPU. Two machines will show the real number.
