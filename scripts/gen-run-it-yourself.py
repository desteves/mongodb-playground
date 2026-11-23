#!/usr/bin/env python3
"""Regenerate docs/run-it-yourself.md from crates/core/src/guide.json (the single source for the guide).

The same guide is served at GET /api/v1/guide, by the MCP tool get_setup_guide, in the web app and as Markdown
at /docs/run-it-yourself.md. Run this after editing guide.json:  python3 scripts/gen-run-it-yourself.py
"""
import json
import re
from pathlib import Path

root = Path(__file__).resolve().parent.parent
g = json.loads((root / "crates/core/src/guide.json").read_text())
lib = (root / "crates/core/src/lib.rs").read_text()
utm = re.findall(r'\("(utm_[a-z]+)",\s*"([^"]+)"\)', lib.split("pub const UTM")[1].split("];")[0])
register = g["atlasRegister"]["url"] + "?" + "&".join(f"{k}={v}" for k, v in utm)

out = [f"# {g['title']}", "", g["summary"], ""]
out += [
    "> Generated from `crates/core/src/guide.json` by `scripts/gen-run-it-yourself.py` (also served at `GET /api/v1/guide`, "
    "by the MCP tool `get_setup_guide`, and as Markdown at `/docs/run-it-yourself.md`). Replace `{origin}` with the site you "
    "are using, for example `https://mongoplayground.com` or `http://localhost:8080`.",
    "",
    f"**Free-forever Atlas cluster (M0, 512 MB, no credit card):** [Create a free Atlas cluster]({register})",
    "",
]
by_id = {o["id"]: o for o in g["options"]}
if g.get("choose"):
    out += ["## Which one should I use?", ""]
    out += [f"- {c['want']}: **{by_id[c['use']]['title']}**" for c in g["choose"]]
    out.append("")
group = None
for o in g["options"]:
    if o.get("group") and o["group"] != group:
        group = o["group"]
        out += [f"## {group}", ""]
    out += [f"### {o['title']}", ""]
    if o.get("summary"):
        out += [o["summary"], ""]
    for i, s in enumerate(o["steps"], 1):
        label = s["label"].replace("{atlasRegister}", register)
        if "command" in s:
            out += [f"{i}. {label}", "", "   ```bash", f"   {s['command']}", "   ```"]
        elif "url" in s:
            out.append(f"{i}. [{label}]({s['url'].replace('{atlasRegister}', register)})")
        else:
            out.append(f"{i}. {label}")
    out.append("")
if g.get("note"):
    out.append(f"_{g['note']}_")
text = "\n".join(out).rstrip("\n") + "\n"
utm_query = "&".join(f"{k}={v}" for k, v in utm)


def tag(m):
    url = m.group(0)
    host = re.match(r"https://([^/?#]+)", url).group(1).lower()
    if (host != "mongodb.com" and not host.endswith(".mongodb.com")) or "utm_campaign=" in url:
        return url
    return url + ("&" if "?" in url else "?") + utm_query


# every link back to mongodb.com carries the same UTM parameters
text = re.sub(r"https://[^\s)>\]`\"']+", tag, text)
(root / "docs/run-it-yourself.md").write_text(text)
print("wrote docs/run-it-yourself.md")
