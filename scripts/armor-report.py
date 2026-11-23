#!/usr/bin/env python3
"""Summarise Cloud Armor's decisions for the playground's load balancer, to decide about enforcement.

While enforce_origin_restriction is false the origin rules only LOG ("preview"). Before enforcing, real traffic
must show:
  - zero "preview DENY"      : every real request carries our Cloudflare origin key, and
  - zero default-rule hits   : every real request comes from a published Cloudflare range.
Anything else listed here is traffic that would be blocked after enforcement (your own direct tests included).

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

total = len(entries)
statuses = collections.Counter()
preview_deny, default_hits = [], []
for e in entries:
    j = e.get("jsonPayload", {}) or {}
    h = e.get("httpRequest", {}) or {}
    statuses[str(h.get("status", "?"))] += 1
    row = (h.get("remoteIp", "?"), h.get("status", "?"), (h.get("requestUrl") or "")[:70], (h.get("userAgent") or "")[:40])
    if (j.get("previewSecurityPolicy") or {}).get("outcome") == "DENY":
        preview_deny.append(row)
    if str((j.get("enforcedSecurityPolicy") or {}).get("priority", "")) == DEFAULT_RULE:
        default_hits.append(row)

print(f"last {hours} h: {total} requests through the load balancer; status codes: {dict(sorted(statuses.items()))}")
print(f"\n1. would be DENIED for a missing or wrong origin key (preview rule 2100): {len(preview_deny)}")
for r in preview_deny[:15]:
    print("   ", *r)
print(f"\n2. reached the default rule, i.e. source NOT in the Cloudflare ranges: {len(default_hits)}")
for r in default_hits[:15]:
    print("   ", *r)

if total == 0:
    print("\nNo requests found: generate some traffic (open the site, run a query) and run this again.")
elif not preview_deny and not default_hits:
    print("\nok: all real traffic carries the origin key and comes from Cloudflare. Safe to enforce.")
else:
    print("\nNOT safe yet. Rows above would be blocked. Your own direct tests (curl --resolve to the load balancer IP) are expected;")
    print("anything from Cloudflare addresses (104.x, 172.x, 2606:4700...) means the origin key or the IP list is wrong.")
