# AGENTS.md

Guidance for AI agents (and humans) working with or on this repository. Agents are first-class users of this
product: every feature must be discoverable and usable through the API.

## What this project is

MongoDB Playground: an online MongoDB shell. Users supply documents, run mongosh-style statements (CRUD,
aggregation, indexes, Atlas Search, Vector Search), see query plans, and share a UUID link. Rust API on Cloud
Run, Svelte 5 + Vite + CodeMirror 6 web UI, Tauri 2 desktop app, Terraform for GCP, Atlas (M10, Private Service
Connect) and Cloudflare.

## Using the playground as an agent

- `GET /llms.txt`: quick start. `GET /api/v1/openapi.json` (also `/openapi.json`): OpenAPI 3.1.
- MCP (Streamable HTTP, stateless): `POST /mcp` or `/api/v1/mcp`; discovery card at `/.well-known/mcp.json`.
  Tools: `run_query`, `validate_query`, `explain_query`, `share_query`, `get_shared`, `export_playground`,
  `list_examples`, `get_setup_guide`, `get_limits`.
- Suggested flow: `get_limits` -> `list_examples` -> `validate_query` -> `run_query` / `explain_query` -> `share_query`.
- Errors are `{code, message, hint}`; follow the `hint`. Rate limits return `429` with `Retry-After`.
- Limits: 8 collections, 256 documents each, 25 statements, 3 indexes + 1 search + 1 vector index per
  collection, 10 s per statement. `$where`, `$function`, `$out`, `$merge` and cross-database references are rejected.

## Working on the code

| Path | What |
|---|---|
| `crates/core` | Parser (`shellvalue.rs`, `shell.rs`), validation, runner, explain, share, export, guide, examples (`examples.json`) |
| `crates/api` | axum server, rate limiting and Turnstile (`guard.rs`), MCP (`mcp.rs`), `openapi.json`, `llms.txt` |
| `src-tauri` | Desktop app; runs `playground-core` in-process |
| `web` | Svelte 5 (runes) UI, Vite build, vitest, Playwright |
| `infra` | Terraform: bootstrap, iam, network, atlas, secrets, runtime, edge, cloudflare, cicd, observability |
| `scripts` | Test and load scripts (all use a throwaway Atlas CLI local deployment) |

Commands:

```bash
cargo fmt --all && cargo clippy --workspace --exclude playground-desktop --all-targets -- -D warnings
cd web && npm run check && npm test && npm run build
scripts/test-with-atlas-local.sh        # Rust tests + HTTP smoke test on a local Atlas deployment (port 6310)
scripts/e2e.sh                          # Playwright (needs `npx playwright install chromium` once)
scripts/loadtest-with-atlas-local.sh    # load test, abuse test, leftover-database check
```

## Rules for changes

1. **Agent parity.** A UI feature needs an API endpoint, an OpenAPI entry, a mention in `llms.txt`, and (where it
   makes sense) an MCP tool. Update `crates/api/src/openapi.json`, `crates/api/src/llms.txt` and `mcp.rs` together.
2. **Tests run against a local Atlas deployment** named `mdbtest` on port 6310. The scripts create and delete it;
   do not point tests at a shared cluster.
3. **Never commit secrets.** `.env`, `*.tfvars`, state and plan files are git-ignored. Credentials go in env vars.
4. **Sample data is behind `FEATURE_SAMPLE_DATA`** and off by default. Templates must not depend on it.
5. **Share links are random UUIDs**, not content hashes. Do not document them as content-addressed.
6. Keep Terraform formatted (`terraform fmt -recursive`) and run `plan` before `apply`.
