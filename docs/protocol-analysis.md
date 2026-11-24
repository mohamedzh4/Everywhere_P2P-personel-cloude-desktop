# Everywhere: what we take from Sunshine and Moonlight

Date: 2026-11-24. Status: analysis, nothing built.

Builds on `../../docs/sunshine-moonlight-analysis.md`, which read the Sunshine,
moonlight-common-c and moonlight-qt source (commits and line numbers there).
This file answers a different question: **what changes when the client is a
browser and the host is your own Windows gaming PC behind a home router.**

Labels: **Measured** = we ran it (see `test-logs/`). **Estimate** = reasoned,
not measured yet.

Licence rule (unchanged): Sunshine and Moonlight are GPL-3.0. We copy no code.
We take ideas, protocol mechanisms and encoder settings, and write our own.

## 1. The one big difference: a browser cannot speak Sunshine's protocol

Moonlight talks to Sunshine with raw UDP (RTP-like video, ENet control), plus
HTTP/HTTPS and RTSP for setup. A web page can do none of that. A browser can
only send real-time data through:

| Browser API | Gets through home NAT / CGNAT? | Verdict for Everywhere |
|---|---|---|
| **WebRTC** (RTP media + SCTP data channels, ICE, DTLS) | **Yes.** ICE does hole punching through NAT and firewalls | **Our transport** |
| WebTransport (QUIC) | No. Needs the PC to be a reachable server with a real TLS certificate; no hole punching | Rejected for version 1 |
| WebSocket (TCP) | Only through a public relay | Last-resort fallback, later |

So Everywhere uses **WebRTC**, and every Sunshine/Moonlight idea is mapped onto
WebRTC's standard pieces instead of a custom UDP protocol. This is also what
browser cloud-gaming services use, so the browser side is well-trodden.

## 2. Two ways to carry video inside WebRTC

| | **A. WebRTC video track (RTP)** | B. Data channel + WebCodecs |
|---|---|---|
| How | Host sends H.264/H.265/AV1 as an RTP track; browser decodes into `<video>` | Host sends Sunshine-style packets on an unreliable data channel; page decodes with WebCodecs, draws on a canvas |
| Hardware decode | Yes (Measured on this Mac: H.264, H.265, VP9 hardware; AV1 software) | Yes (WebCodecs H.264/H.265 hardware Measured; AV1 not) |
| Loss repair | Browser built-in: NACK resend, ULPFEC/FlexFEC, keyframe request (PLI) | Our own Reed-Solomon + RFI, like Sunshine |
| Congestion / bitrate | Standard: transport-wide feedback (TWCC) to the host, host adapts bitrate | Our own; SCTP congestion control sits under it and can throttle |
| Jitter buffer | Browser's; can be pushed to ~0 with `jitterBufferTarget` (Measured: supported) and the playout-delay extension | Ours, Moonlight-style "drop to newest" |
| Code we write | Less | Much more |

**Decision: A first.** It is the proven path, and it gives us most of what
Sunshine does by hand for free. B stays as an experiment we build only if
measurements (plan phase 5) show A is clearly slower.

## 3. Part by part: Sunshine/Moonlight → Everywhere

| Part | Sunshine / Moonlight | Everywhere | Why |
|---|---|---|---|
| Finding the PC | mDNS on LAN, or type an IP | **PC code** (9 digits) → meeting point on public Nostr relays | Works behind CGNAT, no server of ours |
| Pairing | 4-digit PIN once, client certificate kept forever | **Password checked by the PC with a PAKE** (CPace or SPAKE2) every session | Nothing secret is kept on a friend's laptop |
| Session setup | HTTP 47989 + HTTPS 47984 + RTSP 48010 + SDP lines | One encrypted SDP offer/answer through the meeting point | One step instead of three protocols |
| Reaching the PC | User opens ports or uses a VPN; Sunshine can do UPnP | **ICE**: LAN, IPv6, UPnP port, hole punching, tried in parallel | See section 4 |
| Encryption | Control AES-GCM; video encryption optional | **Always**: DTLS-SRTP for video/audio, DTLS for data | Mandatory in WebRTC |
| Video packets | RTP-like + NV header, frame type, encode latency | Standard RTP + H.264/H.265/AV1 packetizer; `abs-capture-time` extension carries the capture time | Same "host latency per frame" stat, standard format |
| FEC | Reed-Solomon, fixed 20 %, off for big frames | ULPFEC/FlexFEC (Measured: listed by the browser) **plus NACK resend** | NACK alone repairs most loss when RTT < ~50 ms (Estimate); FEC % follows measured loss |
| Lost frame recovery | RFI on Windows NVENC, else IDR | **Keyframe on PLI, rate-limited** (max 1 per 200 ms); small VBV so keyframes stay small; later temporal layers | Browsers do not support RFI-style messages (RPSI/LNTF) |
| Bitrate | Fixed for the session | **Adaptive** from TWCC feedback (Google congestion control) | The gap Sunshine leaves open; uni Wi-Fi needs it |
| Pacing | Fixed 800 Mbit/s bursts | Pacer at the estimated rate | Avoids router queue drops |
| Encoder settings | NVENC ultra-low-latency, CBR, no B-frames, VBV = 1 frame; AMF/QSV equivalents | **Same values** (the table in the main analysis §1.5) | Facts about the encoders, not code |
| Capture (Windows) | DXGI Desktop Duplication, Windows Graphics Capture | DXGI Desktop Duplication first (GPU texture straight to the encoder, zero copy) | Works on the lock screen when run as SYSTEM |
| Audio | WASAPI loopback, Opus low-delay CBR, 5 ms | WASAPI loopback, Opus low-delay, 10 ms first (5 ms to test) | Browsers expect 10–20 ms Opus; 5 ms is untested |
| Input | ENet control channel; motion batching; virtual HID | **Two data channels**: unordered, no-resend for mouse motion and sticks (batched like Moonlight); reliable ordered for keys and buttons. Host injects with `SendInput` | Motion "latest wins", keys never lost |
| Mouse/keyboard capture | Native app grabs everything | Pointer Lock (raw movement), Keyboard Lock in fullscreen (Measured: both supported in Chromium) | Esc and Alt+Tab reach the game |
| Gamepad | Virtual Xbox pad on the host (ViGEm on Windows) | Browser Gamepad API (Measured: supported). Host side: later, needs a virtual-pad driver | ViGEmBus is no longer maintained; pick the driver in phase 7 |
| Client render | Own renderer, drop-to-newest pacer, optional tearing | Browser `<video>`, low-latency hints | **Browser costs about one frame** more than native Moonlight (Estimate); 120 fps halves it |
| Admin UI | Web UI on 47990 (attack surface) | **None.** Settings live in the tray app; no listening port | |

## 4. Direct paths, fastest first, and why each is still safe

You asked for "direct when IPv6 or no CGNAT, but still secure". In WebRTC
every path below is tried at the same time; the host picks the working pair
with the lowest measured round-trip time.

| Order | Path | When it works | Speed |
|---|---|---|---|
| 1 | **Same network** (LAN) | Laptop at home | Best |
| 2 | **IPv6 direct** | Both sides have IPv6 | Direct, no NAT at all |
| 3 | **IPv4 with a UPnP-opened port** | Home router allows UPnP and there is no CGNAT or double NAT | Direct |
| 4 | **IPv4 hole punching** (STUN) | Home NAT is "endpoint-independent" (Measured at this Mac's network) | Direct |
| 5 | Relay | Strict networks (symmetric NAT on both ends, or UDP blocked) | Not in version 1; clear message instead; optional own relay later |

**Why direct is still secure:**

- **No open service.** The PC listens on nothing permanent. A UDP port exists
  only during a session, and it answers only ICE checks signed with that
  session's random username and password, which the attacker never sees.
- **IPv6 direct needs no open port either.** Both sides send first, so the home
  router's IPv6 firewall opens a pinhole only for that one laptop, for that
  session.
- **UPnP port (path 3)** is opened per session and closed at the end. Same ICE
  check, so a port scan sees nothing that answers.
- **Everything is encrypted** with DTLS. The DTLS certificate fingerprints
  travel inside the SDP, and the SDP is encrypted with a key that only the
  correct password produces (PAKE). So the meeting point cannot read the
  offer, change it, or sit in the middle.
- **The password is never sent**, not even hashed. A PAKE lets the PC and the
  browser prove they know the same password; an eavesdropper cannot test
  guesses offline. Online guesses are rate-limited by the PC.

## 5. What we skip from Sunshine

Game list and launching apps, several viewers at once, HDR (later), display
mode switching, the web admin UI, the HTTP/RTSP setup protocols, mDNS
(Chromium's own mDNS host candidates cover the LAN case, Measured), and RFI.

## 6. Risks found

1. **Strict networks have no free fix.** If UDP is blocked or both sides are
   symmetric NAT, a relay is the only way, and nobody can run a free video
   relay for everyone. Version 1 says so clearly; an optional "own relay" field
   comes later.
2. **Browser adds latency.** `<video>` goes through the browser's compositor.
   Estimate: ~1 frame more than Moonlight. To measure in phase 5.
3. **Keyframe on loss** (no RFI) means a visible stutter on unrecoverable loss.
   NACK + FEC + small keyframes should make it rare; to measure with forced loss.
4. **Public Nostr relays are other people's servers.** They only see a hash of
   the PC code and encrypted blobs, but they can go down. We ship several and
   refresh the list each release.
5. **Unsigned installer** triggers Windows SmartScreen. Apply to SignPath
   Foundation (free signing for open source) early.
