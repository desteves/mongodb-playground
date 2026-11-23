#!/usr/bin/env python3
"""Compare the Cloudflare IP ranges hard-coded in infra/modules/edge/main.tf with Cloudflare's live lists.

Run this before turning on enforce_origin_restriction: a range missing from the Terraform list would lock
real visitors out once the origin rule is enforced.

Usage: python3 scripts/check-cloudflare-ips.py
Exit status: 0 = the lists match, 1 = differences found, 2 = could not fetch the live lists.
"""
import json
import re
import sys
import urllib.request
from pathlib import Path

SRC = Path(__file__).resolve().parent.parent / "infra" / "modules" / "edge" / "main.tf"
text = SRC.read_text()
block = text[text.index("cloudflare_ipv4"): text.index("# A basic src_ip_ranges")]
v4_re = re.compile(r"\b\d{1,3}(?:\.\d{1,3}){3}/\d{1,2}\b")
v6_re = re.compile(r"\b[0-9a-fA-F]{1,4}(?::[0-9a-fA-F]{0,4}){1,7}/\d{1,3}\b")
have = {"v4": set(v4_re.findall(block)), "v6": {x.lower() for x in v6_re.findall(block)}}


UA = {"User-Agent": "Mozilla/5.0 (compatible; mongodb-playground-ip-check/1.0; +https://github.com/desteves/mongodb-playground)"}


def get(url):
    # Cloudflare answers 403 to Python's default user agent, so send a descriptive one.
    with urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=20) as r:
        return r.read().decode()


def live_lists():
    try:
        return {
            "v4": {l.strip().lower() for l in get("https://www.cloudflare.com/ips-v4").splitlines() if l.strip()},
            "v6": {l.strip().lower() for l in get("https://www.cloudflare.com/ips-v6").splitlines() if l.strip()},
        }
    except Exception as first:  # noqa: BLE001
        # Fallback: the same lists from Cloudflare's public API (no token needed).
        try:
            res = json.loads(get("https://api.cloudflare.com/client/v4/ips"))["result"]
            return {"v4": {c.lower() for c in res["ipv4_cidrs"]}, "v6": {c.lower() for c in res["ipv6_cidrs"]}}
        except Exception as second:  # noqa: BLE001
            raise RuntimeError(f"{first}; fallback API: {second}") from second


try:
    live = live_lists()
except Exception as e:  # noqa: BLE001
    print(f"could not fetch Cloudflare's lists: {e}", file=sys.stderr)
    print("Open https://www.cloudflare.com/ips-v4 and /ips-v6 in a browser and compare them with infra/modules/edge/main.tf by hand.", file=sys.stderr)
    sys.exit(2)

bad = False
for fam in ("v4", "v6"):
    missing = sorted(live[fam] - have[fam])
    extra = sorted(have[fam] - live[fam])
    print(f"IP{fam}: terraform has {len(have[fam])}, Cloudflare publishes {len(live[fam])}")
    if missing:
        bad = True
        print(f"  MISSING from terraform (visitors on these ranges would be blocked): {', '.join(missing)}")
    if extra:
        print(f"  extra in terraform (harmless, no longer published): {', '.join(extra)}")
if bad:
    print("\nAdd the missing ranges to infra/modules/edge/main.tf (cloudflare_ipv4 / cloudflare_ipv6) and apply BEFORE enforcing.")
    sys.exit(1)
print("\nok: every published Cloudflare range is allowed")
