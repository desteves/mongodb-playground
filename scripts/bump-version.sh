#!/usr/bin/env bash
# Set the release version everywhere it is written down, so the files cannot drift apart:
#   scripts/bump-version.sh 0.3.0
# Cargo.toml (workspace), Cargo.lock (the playground-* crates), the OpenAPI document, the desktop app config,
# and the web app's package.json and package-lock.json. It changes nothing else.
set -euo pipefail
cd "$(dirname "$0")/.."
NEW=${1:?usage: scripts/bump-version.sh <major.minor.patch>}
[[ "$NEW" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "not a version: $NEW" >&2; exit 1; }

python3 - "$NEW" <<'PY'
import re, sys, pathlib

new = sys.argv[1]
root = pathlib.Path(".")

def edit(path, pattern, repl, count=1, flags=0):
    p = root / path
    s = p.read_text()
    out, n = re.subn(pattern, repl, s, count=count, flags=flags)
    if n == 0:
        sys.exit(f"{path}: no version found to change")
    p.write_text(out)

# the version we are leaving, from the workspace
cargo = (root / "Cargo.toml").read_text()
m = re.search(r'^version = "(\d+\.\d+\.\d+)"', cargo, re.M)
if not m:
    sys.exit("Cargo.toml: no workspace version")
old = m.group(1)
if old == new:
    sys.exit(f"already at {new}")

edit("Cargo.toml", r'^version = "%s"' % re.escape(old), f'version = "{new}"', flags=re.M)
# every workspace crate in the lockfile (they are named playground-*)
edit("Cargo.lock", r'(name = "playground-[a-z-]+"\nversion = ")%s(")' % re.escape(old), r"\g<1>%s\2" % new, count=0)
edit("crates/api/src/openapi.json", r'("version": ")%s(")' % re.escape(old), r"\g<1>%s\2" % new)
edit("src-tauri/tauri.conf.json", r'("version": ")%s(")' % re.escape(old), r"\g<1>%s\2" % new)
edit("web/package.json", r'("version": ")%s(")' % re.escape(old), r"\g<1>%s\2" % new)
# the lockfile names the root package twice, at the top
edit("web/package-lock.json", r'("version": ")%s(")' % re.escape(old), r"\g<1>%s\2" % new, count=2)
print(f"{old} -> {new}")
PY
git diff --stat -- Cargo.toml Cargo.lock crates/api/src/openapi.json src-tauri/tauri.conf.json web/package.json web/package-lock.json
