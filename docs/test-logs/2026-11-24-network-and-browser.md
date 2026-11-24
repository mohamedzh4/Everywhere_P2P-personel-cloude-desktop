# 2026-11-24: network and browser checks (phase 0)

Machine: the developer's Mac (macOS 26.5.1, Apple Silicon), on Wi-Fi.
**Not the gaming PC.** Run both tools again on the gaming PC and on the
university network before phase 4.

## Network (`tools/network_check.py`)

| Check | Result |
|---|---|
| Public IPv4 | yes |
| Public IPv6 | none (no global IPv6 on the interface, no IPv6 answer) |
| Path to the internet | Mac → router → second router → ISP. **Double NAT** inside the home |
| CGNAT | not detected (the first ISP hop is a public address) |
| NAT mapping (STUN, 2 servers answered, same public port) | **endpoint-independent**: hole punching should work |
| UDP to STUN | works (stun.cloudflare.com and stun1.l.google.com; stun.l.google.com did not answer) |
| TCP 443 out | works |
| STUN round trip | 36–95 ms |

Meaning for Everywhere on this network: path 4 (hole punching) works. Path 3
(UPnP) would not, because of the double NAT: the inner router cannot open a
port on the outer one. Path 2 (IPv6) is not available.

Bugs found in the script on the first run and fixed: "port kept" was reported
as "no NAT"; the IPv6 test used IPv4-mapped addresses; the first traceroute hop
was dropped on macOS; a second home router was reported as possible CGNAT.

## Browser (`tools/browser_check.html`, served on localhost)

Browser: the Claude desktop app's built-in Chromium (Chrome 156).

| Check | Result |
|---|---|
| WebRTC decode 1080p60 | H.264 hardware, H.265 hardware, VP9 hardware, AV1 software |
| WebCodecs hardware decode | H.264 yes, H.265 yes, AV1 no |
| FEC codecs offered | ulpfec, flexfec-03, red, rtx |
| ICE | UDP works; one public IPv4 candidate from two STUN servers (same port) → endpoint-independent; local IPs hidden with mDNS |
| Pointer Lock, Keyboard Lock, Gamepad API, fullscreen | all available |
| `jitterBufferTarget`, `requestVideoFrameCallback`, WebTransport | available |

Opened as a `file:` snapshot first: no secure context, so Keyboard Lock,
WebCodecs and ICE were missing. The real web client will be on HTTPS (GitHub
Pages), so this does not apply to it.

## Not tested yet

- The gaming PC itself (Windows, its router, IPv6, UPnP).
- A university network.
- Anything about streaming: nothing is built.
