#!/usr/bin/env bash
# Render web/public/og.svg to web/public/og.png (1200x630), the Open Graph / social card image.
# Uses the Chromium that Playwright installs for the e2e tests (`cd web && npx playwright install chromium` once).
set -euo pipefail
cd "$(dirname "$0")/.."
node scripts/make-og-image.mjs
