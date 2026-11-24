# Everywhere: plan

Date: 2026-11-24. Status: **phases 1–4 and most of 5 are written** (see the README's status table); not yet run end to end on real networks.

**Everywhere** lets you use your own gaming PC from any browser (university
laptop, a friend's laptop) with nothing installed on that laptop and no server
to run. Open source, free, for people who are not technical.

Read with `protocol-analysis.md` (why WebRTC, and what we take from
Sunshine/Moonlight). Labels: **Measured** / **Estimate** as there.

## 1. What the user does

1. Download `Everywhere-Setup.exe` from the GitHub Releases page, double-click.
2. Click "Allow" on the Windows firewall prompt.
3. The tray app shows a **PC code** (`482-913-007`) and a **password**
   (generated, can be changed), with copy buttons.
4. On any laptop, open **https://mohamedzh4.github.io/Everywhere_P2P-personel-cloude-desktop/**, type
   the code and password, click Connect. Fullscreen, play.

No account, no router settings, no ISP call, no server.

## 2. The pieces (all free, all in one GitHub repo)

```
 Laptop (any browser)                                     Your PC (Windows)
┌──────────────────────────────┐                  ┌──────────────────────────────┐
│ Web client (static page,     │  1. hello, via   │ Everywhere host              │
│ GitHub Pages)                │◀── public Nostr ─▶│  tray app (Tauri)            │
│  code + password → PAKE      │    relays,       │  service (runs as SYSTEM)    │
│  WebRTC: <video>, input      │    encrypted     │   capture DXGI → NVENC/AMF/QSV│
│  channels, gamepad           │                  │   WebRTC (Rust, str0m)        │
└──────────────┬───────────────┘                  │   WASAPI audio → Opus         │
               │   2. direct WebRTC: LAN / IPv6 / │   SendInput                   │
               └──── UPnP / hole punch ──────────▶└──────────────────────────────┘
                      (DTLS-SRTP, always encrypted)
       STUN (Google, Cloudflare, free) only tells each side its public address
```

Implemented choices that differ from the first draft: the PAKE is CPace on
ristretto255 (`host/src/pake.rs`, `web/js/pake.js`); str0m uses Windows CNG
crypto on Windows; the licence is MIT; a local status page
(`http://127.0.0.1:47800`) stands in for the tray app until phase 6.

| Piece | Where | Cost |
|---|---|---|
| Source code | GitHub repo `everywhere` | Free |
| Windows installer | GitHub Releases, built by GitHub Actions | Free |
| Web client | GitHub Pages, same repo | Free |
| Meeting point | Public Nostr relays (several, list shipped in the app) | Free, nobody runs anything for us |
| STUN | Google and Cloudflare public STUN | Free |
| Code signing | SignPath Foundation (open source programme) | Free, needs an application |

## 3. Connection: how a session starts

1. The PC picks a random 9-digit **PC code** once and listens on the relays
   for messages tagged `SHA-256("everywhere/v1/" + code)`. The code is an
   address, not a secret.
2. The browser sends PAKE message A to that tag. The PC answers with B. Both
   now hold the same key **K** only if the password matched. A key
   confirmation message proves it; on failure the PC counts a wrong attempt.
3. The browser sends its WebRTC **offer**, encrypted with K (AES-GCM). The PC
   answers with its **answer**, encrypted with K. Relays see only random
   bytes, so they learn neither IP addresses nor DTLS fingerprints.
4. ICE tries every path at once (section 4). DTLS checks the fingerprints from
   step 3, so no one in the middle can join.
5. Video, audio and input start. Relays are not used again until the next
   session.

**Wrong passwords:** 5 tries, then waits double each time (1 s, 2 s, 4 s…);
after 20 failures in a day the PC stops answering until you click "unlock"
in the tray. The tray shows every attempt and "Connected: <browser>, <city
from IP>" with a **Disconnect** button.

## 4. Connection paths, fastest first

The host is the ICE "controlling" side, so it chooses. It nominates the
working pair with the lowest measured round trip, preferring direct paths.

| # | Path | Needs | Notes |
|---|---|---|---|
| 1 | LAN | Same Wi-Fi | Browser hides its local IP with mDNS; works (Measured: mDNS candidates on) |
| 2 | **IPv6 direct** | IPv6 on both sides | No NAT. The router's IPv6 firewall opens a pinhole because both sides send |
| 3 | **IPv4, UPnP port** | Router with UPnP on, a public IPv4 on the router (no CGNAT, no double NAT) | The host asks the router for a port per session, closes it after. Works even when the laptop's network is strict |
| 4 | **IPv4 hole punching** | At least one side with endpoint-independent NAT | Most home routers. Measured: the network this Mac is on is endpoint-independent |
| 5 | Relay | | **Not in version 1.** The page explains the network blocks direct connections and suggests a phone hotspot. Later: an optional "my own TURN server" setting |

All direct paths have the same security (see `protocol-analysis.md` §4): no
permanent open port, ICE checks with per-session secrets, DTLS everywhere.

## 5. Performance targets

Budget for 1080p60, everything except the network. All **Estimates** until
phase 5 measures them.

| Stage | Where | How | Budget |
|---|---|---|---|
| Capture | PC | DXGI Desktop Duplication, GPU texture, no CPU copy | 1–2 ms |
| Encode | PC | NVENC / AMF / QSV, ultra-low-latency, CBR, no B-frames, VBV ≈ 1 frame | 3–6 ms |
| Packetize + pace | PC | RTP, pacer at the estimated rate | 1–3 ms |
| Network | | | RTT / 2 (not ours) |
| Jitter buffer | browser | `jitterBufferTarget = 0`, playout-delay 0 | 0–5 ms |
| Decode | browser | Hardware H.264 (Measured: available) | 2–5 ms |
| Display | browser | `<video>` through the compositor | 8–17 ms at 60 Hz |
| **Added total** | | | **≈ 15–38 ms + network** |

Rules to hit the targets:

- **H.264 is the default** (hardware decode everywhere). H.265 or AV1 only when
  the browser says hardware decode is available (Measured on this Mac: H.265
  yes, AV1 software only).
- **Bitrate adapts** to the network from transport-wide congestion feedback.
  Start at 15 Mbit/s for 1080p60 and let it move between 3 and 50.
- **Loss:** NACK resend first, FEC that follows measured loss, a keyframe only
  when needed and at most one per 200 ms.
- **120 fps** when the laptop's screen supports it: halves the display wait.
- **Live stats overlay** (like Moonlight's): path type (LAN/IPv6/UPnP/punched),
  RTT, bitrate, loss, decode time, frames dropped.

## 6. Security summary

| Threat | Protection |
|---|---|
| Someone guesses the password | PAKE (no offline guessing), rate limit, daily lock, tray alerts |
| A relay or Wi-Fi owner reads or changes the hello | Offer and answer are encrypted and authenticated with the PAKE key |
| Someone in the middle of the stream | DTLS fingerprints checked against the authenticated SDP |
| Port scans of your PC | Nothing listens. Session ports answer only signed ICE checks; UPnP ports close at session end |
| Friend's laptop keeps access | Nothing is stored; closing the tab ends the session; change password in the tray |
| Bad update | Signed installer (SignPath), updates checked against the release signature |
| Malicious web page copy | Users are told the one official URL; the page is built from the public repo by GitHub Actions |

## 7. Build phases

Each phase ends with a test that runs on real hardware, written in
`test-logs/`.

| Phase | What | Done when |
|---|---|---|
| **0** | Network and browser checks (`tools/`) | **Done 2026-11-24** on this Mac; still to run on the gaming PC and at the university |
| 1 | Stream on LAN: capture → NVENC H.264 → WebRTC → browser `<video>`. Offer/answer copied by hand | Desktop visible in the browser at 1080p60; latency measured |
| 2 | Input: mouse and keyboard channels, Pointer Lock, Keyboard Lock | A game is playable on the LAN |
| 3 | Meeting point: PC code, PAKE, encrypted offer/answer on Nostr relays; web client on GitHub Pages | Connect from a phone hotspot, no manual copying |
| 4 | Path ladder: IPv6, UPnP, hole punching; path shown in the overlay; "can't connect here" message | Each path forced and seen working once |
| 5 | Performance: adaptive bitrate, FEC, keyframe limits, audio, 120 fps; A/B against Sunshine + Moonlight on the same PC | Numbers in a test log; within one frame of Moonlight |
| 6 | Install and run like a product: installer, Windows service (lock screen, admin prompts), tray, auto-start, signing, updates | A non-technical friend installs and connects without help |
| Later | Gamepad (host driver), virtual display (no monitor), part-of-PC VM using the main project's hypervisor code, Linux host, optional own relay | |

## 8. Technology

| Part | Choice | Why |
|---|---|---|
| Host app | Rust + Tauri (reuse the main project's host-agent shell, tray and installer setup) | Already in the repo, small installer |
| WebRTC on the host | `str0m` (Rust, sans-IO) | We control the pacer, ICE nomination and encoder feedback. `webrtc-rs` as fallback |
| Capture | DXGI Desktop Duplication via `windows-rs` | Zero copy to the encoder |
| Encode | FFmpeg (LGPL build) first, NVENC SDK directly later | Fast start, then full control |
| Audio | WASAPI loopback + Opus | |
| Web client | TypeScript + Vite, static, GitHub Pages | No backend |
| Meeting point | `nostr-tools` (browser), `nostr-sdk` (Rust) | Both sides exist and are maintained |
| PAKE | CPace or SPAKE2 with **one** shared test-vector suite run against both the Rust and the JS side | Both sides must agree byte for byte |
| Licence | MIT or Apache-2.0 for Everywhere | Easy for others to use; no GPL code copied |

## 9. Decisions waiting for you

1. **Video path A** (WebRTC video track) first, B (data channel + WebCodecs)
   only if A measures too slow.
2. **No relay in version 1**, with a clear "this network blocks direct
   connections" message.
3. **Host is Windows only** at first, whole desktop, no VM.
4. **Licence**: MIT or Apache-2.0.
5. **Repo**: a new public GitHub repo `everywhere` (so Pages gives
   https://mohamedzh4.github.io/Everywhere_P2P-personel-cloude-desktop/), with this folder as the starting point. Done: https://github.com/mohamedzh4/Everywhere_P2P-personel-cloude-desktop
