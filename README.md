# Everywhere

Use your own gaming PC from any browser, anywhere. No server, no account,
nothing to install on the laptop. Free and open source (MIT).

**Status: all parts written, compiled and unit-tested; not yet run end to end
on real networks.** See [Status](#status).

## How it works

```
 Laptop (any browser)                                   Your PC (Windows)
┌──────────────────────────┐   1. hello, through    ┌──────────────────────────┐
│ web/  (GitHub Pages)     │◀── public Nostr ──────▶│ host/  Everywhere.exe     │
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
| `host/` | The PC program (Rust). `src/`: `signaling` (meeting point), `pake`, `nostr`, `session` (WebRTC, str0m), `net` (STUN, UPnP, IPv6), `source` (screen capture), `encoder`, `audio`, `input`, `status` (local page), `config` |
| `web/` | The browser client, static files for GitHub Pages. `js/`: `app`, `connect`, `pake`, `nostr`, `input`, `stats`. `vendor/`: noble crypto, pinned |
| `web/test/` | `node test/run.mjs`; `vectors.json` pins Rust and JS crypto to the same bytes |
| `tools/` | `network_check.py`, `browser_check.html`: what a network allows |
| `docs/` | Plan, protocol analysis, Windows guide, test logs |
| `.github/workflows/` | CI (Linux, Windows, web), Pages deploy, Windows release on tags |

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

Windows-only code (screen capture, input injection) can be compile-checked
from another OS:

```bash
cd host && cargo check --target x86_64-pc-windows-msvc --no-default-features
```

## Publishing

Repo: https://github.com/mohamedzh4/Everywhere_P2P-personel-cloude-desktop

1. Settings > Pages > Source: **GitHub Actions**. Pushing to `main` publishes
   `web/` at https://mohamedzh4.github.io/Everywhere_P2P-personel-cloude-desktop/. A fork under another name must change
   `DEFAULT_WEB_URL` in `host/src/config.rs` and the source link in
   `web/index.html`.
2. `git tag v0.1.0 && git push --tags` builds `Everywhere.exe` and attaches it to
   a release.

## Status

| Part | State |
|---|---|
| Password handshake, sealing, Nostr events | Done; Rust and JS tests agree byte for byte |
| Meeting point (relays) | Done; not yet run against live relays |
| WebRTC stream, playout delay 0, pacing check | Done; measured on one Mac (41 ms median, see test log) |
| Paths: LAN, IPv6, STUN hole punching, UPnP | Done; not yet run across two real networks |
| Input (keyboard, mouse, wheel), Windows injection | Done; compile-checked for Windows, not run |
| Screen capture (DXGI), audio (WASAPI + Opus) | Done; compile-checked for Windows, not run |
| Adaptive bitrate | Done (software encoder is rebuilt on big changes) |
| Status page, config, rate limits | Done; unit-tested |
| Web client | Done; loads cleanly under its CSP; not yet connected to a real host |
| Hardware encoding (NVENC/AMF/QSV), gamepad, installer, Windows service, tray, signing | Next |
