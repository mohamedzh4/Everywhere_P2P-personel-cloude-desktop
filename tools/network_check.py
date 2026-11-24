#!/usr/bin/env python3
"""Everywhere network check: which connection paths will work from this machine.

Run on the home PC (the host) and on a client network (university, friend's
Wi-Fi). Standard library only, works on Windows, macOS and Linux:

    python3 network_check.py
    python3 network_check.py --json   # machine-readable

What it tests:
  1. Public IPv4 and IPv6 (via Cloudflare's /cdn-cgi/trace).
  2. Global IPv6 on a local interface (direct path possible without NAT).
  3. CGNAT: a 100.64.0.0/10 address on an interface, or a private/shared hop
     right after the home router.
  4. NAT mapping behaviour with STUN: one local UDP socket asks two STUN servers
     (different IPs) for its public address. Same public port for both means
     endpoint-independent mapping, so WebRTC hole punching usually works.
     Different ports means symmetric NAT: hole punching will often fail.
  5. Outbound UDP to 3478 and 19302 (STUN ports) and TCP 443 reachability.

It sends nothing personal: only STUN binding requests and an HTTPS GET.
"""

from __future__ import annotations

import ipaddress
import json
import os
import platform
import re
import secrets
import socket
import ssl
import struct
import subprocess
import sys
import time
import urllib.request

STUN_SERVERS = [
    ("stun.l.google.com", 19302),
    ("stun.cloudflare.com", 3478),
    ("stun1.l.google.com", 19302),
]
TRACE_V4 = "https://1.1.1.1/cdn-cgi/trace"
TRACE_V6 = "https://[2606:4700:4700::1111]/cdn-cgi/trace"
CGNAT = ipaddress.ip_network("100.64.0.0/10")
MAGIC = 0x2112A442


def http_trace(url: str, timeout: float = 5.0) -> dict | None:
    try:
        ctx = ssl.create_default_context()
        # The IP-literal URLs present a cert for one.one.one.one; skip host check.
        ctx.check_hostname = False
        ctx.verify_mode = ssl.CERT_NONE
        req = urllib.request.Request(url, headers={"User-Agent": "everywhere-netcheck"})
        with urllib.request.urlopen(req, timeout=timeout, context=ctx) as r:
            body = r.read().decode()
        return dict(line.split("=", 1) for line in body.splitlines() if "=" in line)
    except Exception:
        return None


def local_addresses() -> list[str]:
    """Best-effort list of local IPs, from the OS tool (no extra packages)."""
    cmd = ["ipconfig"] if os.name == "nt" else (["ip", "addr"] if sys.platform.startswith("linux") else ["ifconfig"])
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=5).stdout
    except Exception:
        return []
    found = set()
    for tok in re.findall(r"[0-9a-fA-F:.%]+", out):
        tok = tok.split("%")[0].strip(".")
        try:
            found.add(str(ipaddress.ip_address(tok)))
        except ValueError:
            pass
    return sorted(found)


def stun_request(sock: socket.socket, host: str, port: int, timeout: float = 2.0):
    """Send one STUN Binding Request, return (public_ip, public_port, rtt_ms) or None."""
    try:
        infos = socket.getaddrinfo(host, port, sock.family, socket.SOCK_DGRAM)
    except OSError:
        return None
    # Skip IPv4-mapped IPv6 results (::ffff:a.b.c.d): they are IPv4 in disguise.
    infos = [i for i in infos if not (sock.family == socket.AF_INET6 and i[4][0].startswith("::ffff:"))]
    if not infos:
        return None
    addr = infos[0][4]
    tid = secrets.token_bytes(12)
    msg = struct.pack("!HHI12s", 0x0001, 0, MAGIC, tid)
    sock.settimeout(timeout)
    t0 = time.perf_counter()
    try:
        sock.sendto(msg, addr)
        while True:
            data, _ = sock.recvfrom(2048)
            if len(data) >= 20 and data[8:20] == tid:
                break
    except OSError:
        return None
    rtt = (time.perf_counter() - t0) * 1000
    pos, end = 20, 20 + struct.unpack("!H", data[2:4])[0]
    while pos + 4 <= end:
        atype, alen = struct.unpack("!HH", data[pos:pos + 4])
        val = data[pos + 4:pos + 4 + alen]
        if atype in (0x0020, 0x0001) and len(val) >= 8:  # XOR-MAPPED / MAPPED
            fam, xport = val[1], struct.unpack("!H", val[2:4])[0]
            raw = val[4:8] if fam == 1 else val[4:20]
            if atype == 0x0020:
                xport ^= MAGIC >> 16
                key = struct.pack("!I", MAGIC) + (tid if fam == 2 else b"")
                raw = bytes(b ^ key[i % len(key)] for i, b in enumerate(raw)) if fam == 1 else \
                    bytes(b ^ (struct.pack("!I", MAGIC) + tid)[i] for i, b in enumerate(raw))
            return str(ipaddress.ip_address(raw)), xport, round(rtt, 1)
        pos += 4 + alen + ((4 - alen % 4) % 4)
    return None


def stun_mapping(family: int, local_ips: list[str]) -> dict:
    """Ask several STUN servers from ONE local socket; compare mapped ports."""
    try:
        sock = socket.socket(family, socket.SOCK_DGRAM)
        if family == socket.AF_INET6:
            sock.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 1)
        sock.bind(("::" if family == socket.AF_INET6 else "0.0.0.0", 0))
    except OSError as e:
        return {"ok": False, "error": str(e)}
    local_port = sock.getsockname()[1]
    results = []
    for host, port in STUN_SERVERS:
        r = stun_request(sock, host, port)
        results.append({"server": f"{host}:{port}", "mapped": None if r is None else f"{r[0]}:{r[1]}",
                        "rtt_ms": None if r is None else r[2]})
    sock.close()
    mapped = [r["mapped"] for r in results if r["mapped"]]
    if not mapped:
        verdict = "no UDP answer (UDP blocked, or no route for this IP family)"
    elif len(set(mapped)) == 1:
        ip = mapped[0].rsplit(":", 1)[0]
        verdict = ("no NAT on this path (public address is on this machine)" if ip in local_ips
                   else "endpoint-independent mapping: hole punching should work")
        if len(mapped) == 1:
            verdict += " (only one server answered, not confirmed)"
    else:
        verdict = "symmetric NAT (port changes per server): hole punching will often fail"
    return {"ok": bool(mapped), "local_port": local_port, "servers": results, "verdict": verdict}


def tcp_ok(host: str, port: int, timeout: float = 3.0) -> bool:
    try:
        with socket.create_connection((host, port), timeout=timeout):
            return True
    except OSError:
        return False


def router_hops() -> list[str]:
    """First 3 hops to 1.1.1.1, to spot a CGNAT hop after the home router."""
    if os.name == "nt":
        cmd = ["tracert", "-d", "-h", "3", "-w", "1000", "1.1.1.1"]
    else:
        cmd = ["traceroute", "-n", "-m", "3", "-q", "1", "-w", "1", "1.1.1.1"]
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=20).stdout
    except Exception:
        return []
    hops = []
    for line in out.splitlines():
        if "traceroute to" in line or "Tracing route" in line:
            continue
        m = re.findall(r"\b(\d{1,3}(?:\.\d{1,3}){3})\b", line)
        if m and m[-1] != "1.1.1.1":
            hops.append(m[-1])
    return hops[:3]


def main() -> int:
    report: dict = {"machine": platform.platform()}

    t4, t6 = http_trace(TRACE_V4), http_trace(TRACE_V6)
    report["public_ipv4"] = t4.get("ip") if t4 else None
    report["public_ipv6"] = t6.get("ip") if t6 else None
    report["country"] = (t4 or t6 or {}).get("loc")

    addrs = local_addresses()
    ips = [ipaddress.ip_address(a) for a in addrs]
    report["local_global_ipv6"] = [str(i) for i in ips if i.version == 6 and i.is_global]
    report["local_ipv4"] = [str(i) for i in ips if i.version == 4 and not i.is_loopback
                            and not str(i).startswith("255.") and not str(i).endswith(".255")
                            and str(i) != "0.0.0.0"]

    hops = router_hops()
    report["nat_layers"] = sum(1 for h in hops if ipaddress.ip_address(h).is_private or ipaddress.ip_address(h) in CGNAT)
    report["first_hops"] = hops
    cgnat_hop = any(ipaddress.ip_address(h) in CGNAT for h in hops)
    cgnat_iface = any(ipaddress.ip_address(a) in CGNAT for a in report["local_ipv4"])
    # 192.168.x.x after the first router is usually a second home router (double
    # NAT). 10.x / 172.16-31.x after the first router is often the ISP's CGNAT.
    private_after_router = any(ipaddress.ip_address(h).is_private and not h.startswith("192.168.")
                               for h in hops[1:])
    report["cgnat"] = ("likely" if (cgnat_hop or cgnat_iface) else
                       "possible (private hop after router)" if private_after_router else "not detected")

    report["stun_ipv4"] = stun_mapping(socket.AF_INET, addrs)
    report["stun_ipv6"] = stun_mapping(socket.AF_INET6, addrs) if socket.has_ipv6 else {"ok": False}
    report["tcp_443"] = tcp_ok("1.1.1.1", 443)

    # Which Everywhere paths look possible from here.
    paths = []
    if report["public_ipv6"] and report["local_global_ipv6"]:
        paths.append("direct IPv6 (no NAT; firewall hole punched by WebRTC)")
    v4 = report["stun_ipv4"]
    if v4.get("ok"):
        if "no NAT" in v4["verdict"]:
            paths.append("direct IPv4 (public address on this machine)")
        elif "endpoint-independent" in v4["verdict"]:
            paths.append("IPv4 hole punching (STUN) - direct, good latency")
        else:
            paths.append("IPv4 behind symmetric NAT - works only if the OTHER side is easy")
    if not v4.get("ok") and not report["stun_ipv6"].get("ok"):
        paths.append("UDP blocked: only a TCP/443 relay would work from here")
    report["paths"] = paths

    if "--json" in sys.argv:
        print(json.dumps(report, indent=2))
        return 0

    print(f"Everywhere network check  ({report['machine']})\n")
    print(f"Public IPv4 ........ {report['public_ipv4'] or 'none'}")
    print(f"Public IPv6 ........ {report['public_ipv6'] or 'none'}")
    print(f"Global IPv6 on NIC . {', '.join(report['local_global_ipv6']) or 'none'}")
    print(f"First hops ......... {' -> '.join(hops) or 'unknown'}")
    print(f"Private hops ....... {report['nat_layers']} (2 or more = double NAT, fine for STUN, bad for UPnP)")
    print(f"CGNAT .............. {report['cgnat']}")
    for fam in ("stun_ipv4", "stun_ipv6"):
        s = report[fam]
        print(f"\nSTUN {fam[-4:]}: {s.get('verdict', s.get('error', 'not available'))}")
        for r in s.get("servers", []):
            print(f"   {r['server']:<28} -> {r['mapped'] or 'no answer'}"
                  + (f"  ({r['rtt_ms']} ms)" if r["rtt_ms"] else ""))
    print(f"\nTCP 443 outbound ... {'ok' if report['tcp_443'] else 'blocked'}")
    print("\nPaths that look possible from this machine:")
    for p in paths:
        print(f"   - {p}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
