# Everywhere

Use your own PC (Windows, Mac or Linux) from any browser, anywhere. No
server, no account, nothing to install on the laptop. Free and open source (MIT).

**Downloads:** [Releases](https://github.com/mohamedzh4/Everywhere_P2P-personel-cloude-desktop/releases) ·
install guides for [Windows](docs/install-windows.md), [Mac](docs/install-mac.md), [Linux](docs/install-linux.md).

**Status: works end to end on one network; first tests across real networks
are under way.** See [Status](#status).

## How it works

```
 Laptop (any browser)                           Your PC (Windows, Mac, Linux)
┌──────────────────────────┐   1. hello, through    ┌──────────────────────────┐
│ web/  (GitHub Pages)     │◀── public Nostr ──────▶│ host/  Everywhere app     │
│ PC code + password       │    relays, sealed      │ password check (PAKE)     │
│ WebRTC player + input    │                        │ capture → H.264 → WebRTC  │
└────────────┬─────────────┘                        └────────────▲─────────────┘
             └──── 2. direct, encrypted: LAN / IPv6 / UPnP / hole punch ┘
```

1. The PC listens on a few public Nostr relays for its PC code's topic.
2. The browser and PC run a password handshake (CPace on ristretto255) through
   the relays. The password is never sent; the relays see only sealed bytes.
3. The WebRTC offer and answer travel sealed with the handshake key, so nobody
   in the middle can swap the encryption keys of the stream.
4. WebRTC tries every path at once (same network, IPv6, a UPnP port, hole
   punching) and uses the fastest that works. Video, sound and input go
   straight between the two machines.

Details: [docs/plan.md](docs/plan.md), [docs/protocol-analysis.md](docs/protocol-analysis.md).

## Layout

| Folder | What |
|---|---|
| `host/` | The PC program (Rust). `src/`: `signaling` (meeting point), `pake`, `nostr`, `session` (WebRTC, str0m), `net` (STUN, UPnP, IPv6, port prediction), `source` + `capture_mac`/`capture_x11`/`sck` (screen), `encoder`, `audio`, `input`, `permissions`, `ui` (app window), `status` (its page), `logbook`, `config` |
| `web/` | The browser client, static files for GitHub Pages. `js/`: `app`, `connect`, `pake`, `nostr`, `input`, `stats`. `vendor/`: noble crypto, pinned |
| `web/test/` | `node test/run.mjs`; `vectors.json` pins Rust and JS crypto to the same bytes |
| `tools/` | `network_check.py`, `browser_check.html`: what a network allows |
| `packaging/macos/` | `make-app.sh`: wraps the host into `Everywhere.app` |
| `docs/` | Plan, protocol analysis, install guides, test logs |
| `.github/workflows/` | CI (Linux, macOS, Windows, web), Pages deploy, release for all three on tags |

## Build and test

```bash
cd host && cargo test --release && cargo build --release
```

```bash
cd web && node test/run.mjs
```

Run the host:

```bash
host/target/release/everywhere-host
```

Other commands: `everywhere-host show` (code and password), `new-password`,
and `lan` (test page served by the PC itself, same network only, with a
latency stamp; see `docs/test-logs/`).

Windows-only code can be compile-checked from another OS:

```bash
cd host && cargo check --target x86_64-pc-windows-msvc --no-default-features
```

On a Mac, `packaging/macos/make-app.sh` builds `dist/Everywhere.app`; run that
(not the bare binary) so macOS asks for the Screen Recording and Accessibility
permissions for Everywhere itself. Set `EVERYWHERE_NO_PROMPT=1` to skip the
permission prompts in test runs.

## Publishing

Repo: https://github.com/mohamedzh4/Everywhere_P2P-personel-cloude-desktop

1. Settings > Pages > Source: **GitHub Actions**. Pushing to `main` publishes
   `web/` at https://mohamedzh4.github.io/Everywhere_P2P-personel-cloude-desktop/. A fork under another name must change
   `DEFAULT_WEB_URL` in `host/src/config.rs` and the source link in
   `web/index.html`.
2. `git tag v0.1.2 && git push origin v0.1.2` builds Windows, macOS and Linux
   downloads and attaches them to one release.

## Status

| Part | State |
|---|---|
| Password handshake, sealing, Nostr events | Done; Rust and JS tests agree byte for byte |
| Meeting point (relays), WebRTC stream, playout delay 0 | Done; run end to end on one network (41 ms median, see test log) |
| Paths: LAN, IPv6, STUN, port prediction for strict NATs, UPnP | Done; first test across networks (phone hotspot) under way |
| Windows: screen (DXGI, with pointer), input (SendInput), sound (WASAPI) | Done; the exe runs; pointer and window not yet seen on Windows |
| macOS: screen + sound (ScreenCaptureKit), input (CGEvent), permissions | Done; runs, falls back cleanly without permission; real-screen test pending |
| Linux X11: screen (with pointer), input (XTest), sound (parec) | Done; built and tested in a container. Wayland: later |
| App window (glass), stop/start sharing, activity log | Done (WebView2 / WKWebView; Linux opens the browser) |
| Adaptive bitrate, rate limits on wrong passwords | Done |
| Hardware encoding (NVENC/AMF/QSV/VideoToolbox/VAAPI), gamepad, Wayland, signing | Next |
