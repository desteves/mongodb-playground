#!/usr/bin/env python3
"""Summarise Cloud Armor's decisions for the playground's load balancer, to decide about enforcement.

While enforce_origin_restriction is false the origin rules only LOG ("preview"). Before enforcing, real traffic
must show:
  - zero "preview DENY"      : every real request carries our Cloudflare origin key, and
  - zero default-rule hits   : every real request comes from a published Cloudflare range.
Anything else listed here is traffic that would be blocked after enforcement (your own direct tests included).

Requests are sorted by who sent them, because the load balancer's public IP is scanned all day:
  - real traffic : arrived from a Cloudflare range for our domain. If any of it lacks the origin key, DO NOT enforce.
  - bypass       : not from a Cloudflare range (scanners, your own curl --resolve tests). Enforcing blocks these, which is the point.

Usage: python3 scripts/armor-report.py [hours=24] [project=playground-515215]
"""
import collections
import json
import subprocess
import sys

hours = sys.argv[1] if len(sys.argv) > 1 else "24"
project = sys.argv[2] if len(sys.argv) > 2 else "playground-515215"
DEFAULT_RULE = "2147483647"

out = subprocess.run(
    [
        "gcloud", "logging", "read",
        'resource.type="http_load_balancer" AND resource.labels.url_map_name="playground"',
        f"--project={project}", f"--freshness={hours}h", "--limit=5000", "--format=json",
    ],
    capture_output=True, text=True,
)
if out.returncode != 0:
    sys.exit(out.stderr.strip() or "gcloud logging read failed")
entries = json.loads(out.stdout or "[]")

import ipaddress
import pathlib
import re
from urllib.parse import urlparse

tf = pathlib.Path(__file__).resolve().parent.parent / "infra" / "modules" / "edge" / "main.tf"
CF = [ipaddress.ip_network(c) for c in re.findall(r'"([0-9a-fA-F:.]+/\d+)"', tf.read_text())]


def from_cloudflare(ip):
    try:
        a = ipaddress.ip_address(ip)
    except ValueError:
        return False
    return any(a in n for n in CF if n.version == a.version)


def host_kind(url):
    host = urlparse(url or "").hostname or ""
    try:
        ipaddress.ip_address(host)
        return "raw-ip"  # someone connected to the load balancer's IP, not to our domain
    except ValueError:
        return "domain"


total = len(entries)
statuses = collections.Counter()
by_kind_status = collections.Counter()
problem, bypass, default_hits, fivexx = [], [], [], []
for e in entries:
    j = e.get("jsonPayload", {}) or {}
    h = e.get("httpRequest", {}) or {}
    status = str(h.get("status", "?"))
    url = h.get("requestUrl") or ""
    ip = h.get("remoteIp", "?")
    kind = host_kind(url)
    statuses[status] += 1
    by_kind_status[(kind, status)] += 1
    row = (e.get("timestamp", "")[:19], ip, status, url[:60], (h.get("userAgent") or "")[:30])
    denied = (j.get("previewSecurityPolicy") or {}).get("outcome") == "DENY"
    if denied:
        (problem if from_cloudflare(ip) else bypass).append(row)
    if str((j.get("enforcedSecurityPolicy") or {}).get("priority", "")) == DEFAULT_RULE:
        default_hits.append(row)
    if status.startswith("5"):
        fivexx.append((kind,) + row)

print(f"last {hours} h: {total} requests through the load balancer")
print("status codes by target:")
for kind in ("domain", "raw-ip"):
    print(f"   {kind:7}", dict(sorted((s, n) for (k, s), n in by_kind_status.items() if k == kind)))

print(f"\n1. REAL TRAFFIC the origin-key rule would deny (from a Cloudflare range, missing or wrong key): {len(problem)}")
if problem:
    times = sorted(r[0] for r in problem)
    print(f"    all between {times[0]} and {times[-1]} UTC. If that is long ago (before a rule or deploy changed), run again with a short window:")
    print("    python3 scripts/armor-report.py 2")
for r in problem[:15]:
    print("   ", *r)
print(f"\n2. bypass: would be denied, not from a Cloudflare range (scanners, your own direct tests): {len(bypass)}")
for r in bypass[:8]:
    print("   ", *r)
print(f"\n3. reached the default rule (source outside the Cloudflare ranges): {len(default_hits)}")
if fivexx:
    times = sorted(r[1] for r in fivexx)
    print(f"\n5xx responses: {len(fivexx)}, between {times[0]} and {times[-1]} UTC; examples, and the most common targets:")
    for r in (fivexx[:3] + fivexx[-3:]):
        print("   ", *r)
    for (kind, url), n in collections.Counter((r[0], r[4]) for r in fivexx).most_common(5):
        print(f"    {n:5} {kind} {url}")

if total == 0:
    print("\nNo requests found: generate some traffic (open the site, run a query) and run this again.")
elif problem:
    print("\nNOT safe: real Cloudflare traffic is missing the origin key. Check the Cloudflare 'Origin key' rule before enforcing.")
else:
    print("\nok for stage 1 (enforce_origin_key): no real traffic would be denied. The bypass rows are what the lock is for.")
    if default_hits and any(r for r in default_hits if from_cloudflare(r[1])):
        print("but some default-rule hits come from Cloudflare addresses: the IP list is stale, so wait on stage 2.")
    else:
        print("stage 2 (enforce_origin_restriction) looks safe too: no default-rule hits from Cloudflare addresses.")
