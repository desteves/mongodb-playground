#!/usr/bin/env python3
"""Make the apex and www point only at the load balancer.

Deletes every A, AAAA or CNAME record on <domain> and www.<domain> unless it is an A record whose content
is the load balancer's IP. Anything else on those names (a parked page's address, a CNAME to a hosting
provider) would send some visitors elsewhere and stops Google from validating the load balancer's
certificate. Other record types (TXT for SPF/DKIM/DMARC, MX, CAA) and other names (such as
_domainconnect) are left alone.

Usage: cleanup-dns.py <zone id> <domain> <load balancer ip>     (needs CLOUDFLARE_API_TOKEN)
Run by Terraform (modules/cloudflare, variable remove_conflicting_dns); safe to run by hand.
"""
import json
import os
import sys
import urllib.request

zone, domain, ip = sys.argv[1:4]
token = os.environ.get("CLOUDFLARE_API_TOKEN")
if not token:
    sys.exit("CLOUDFLARE_API_TOKEN is not set (load .env, or set remove_conflicting_dns = false)")

API = f"https://api.cloudflare.com/client/v4/zones/{zone}/dns_records"
names = {domain, f"www.{domain}"}


def call(method, url):
    req = urllib.request.Request(url, method=method, headers={"Authorization": f"Bearer {token}"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.load(r)


records = call("GET", API + "?per_page=100")
if not records.get("success"):
    sys.exit(f"could not list DNS records: {records.get('errors')}")

removed = 0
for r in records["result"]:
    if r["name"] not in names or r["type"] not in ("A", "AAAA", "CNAME"):
        continue
    if r["type"] == "A" and r["content"] == ip:
        continue
    out = call("DELETE", f"{API}/{r['id']}")
    if not out.get("success"):
        sys.exit(f"could not delete {r['type']} {r['name']} -> {r['content']}: {out.get('errors')}")
    print(f"deleted {r['type']} {r['name']} -> {r['content']}")
    removed += 1

print(f"DNS check done: {removed} conflicting record(s) removed; {sorted(names)} point only at {ip}")
