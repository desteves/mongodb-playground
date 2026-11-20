# MongoDB Playground

**An online MongoDB shell for builders: humans and AI agents alike.** Paste your own documents, run mongosh-style
statements (CRUD, aggregation, indexes, Atlas Search, Vector Search), read the query plan, and share the result
with a link. No account, no cluster to set up. Each run uses a throwaway database that is deleted afterwards.

Everything a person can click, an agent can call: a versioned REST API (`/api/v1`), an OpenAPI 3.1 spec,
`llms.txt`, and an MCP server. See [AGENTS.md](AGENTS.md) if you are an agent (or are pointing one at this repo).

| You are | Start here |
|---|---|
| A person who wants to try a query | The hosted site (a Playground with Data, Shell and Results panels), or [Run it locally](#run-it-locally) |
| An agent or tool author | [AGENTS.md](AGENTS.md), [`llms.txt`](llms.txt), `GET /api/v1/openapi.json`, `POST /mcp` |
| Someone self-hosting or contributing | [CONTRIBUTING.md](CONTRIBUTING.md), [docs/run-it-yourself.md](docs/run-it-yourself.md), [infra/README.md](infra/README.md) |
| Curious about the design | [docs/MASTER_PLAN.md](docs/MASTER_PLAN.md) |

**What it is:** a Rust API (axum + the MongoDB driver) on Cloud Run, a Svelte 5 UI built with Vite and
CodeMirror 6, a Tauri 2 desktop app that reuses the same Rust core, and Terraform for GCP, MongoDB Atlas
(M10 over Private Service Connect) and Cloudflare. Keywords: online MongoDB shell, MongoDB playground, test
MongoDB queries, mongosh in the browser, aggregation pipeline tester, Atlas Search and Vector Search sandbox,
MongoDB MCP server.

Status: the core, API, web UI, MCP server, share links, explain, stats, Terraform and the Tauri app are all
written. Rust, HTTP smoke, browser, load and abuse tests pass locally against an Atlas CLI local deployment; see
["Not verified yet"](#not-verified-yet) for what has not been exercised against real accounts.

## Run it locally

You need Rust (rustup), Node 22+, and a MongoDB.

1. Start a MongoDB. Pick one:
   - Docker: `docker compose up mongo`
   - Atlas CLI local deployment: `atlas deployments setup playground --type local`
   - Atlas free cluster (M0): `atlas setup`, then put its connection string in `MONGODB_URI`
2. API: `MONGODB_URI=mongodb://localhost:27017 cargo run -p playground-api` (listens on :8080)
3. Web: `cd web && npm install && npm run dev` (http://localhost:5173, proxies `/api` to :8080)

Tests: `cargo test` (unit tests always run; integration tests need `MONGODB_URI`). `scripts/test-with-atlas-local.sh` creates a throwaway Atlas CLI local deployment (port 6310), runs the unit and integration tests and an HTTP smoke test, and deletes it. `python3 scripts/smoke.py [base-url]` smoke-tests an already running API.

Atlas CLI flags change between releases; check `atlas <command> --help` before relying on the commands above.

## For agents

- `GET /llms.txt`: quick start. `GET /api/v1/openapi.json` (also `/openapi.json`): OpenAPI 3.1.
- `POST /api/v1/mcp` (also `/mcp`): MCP server (tools `run_query`, `validate_query`, `list_examples`, `get_limits`).
- `POST /api/v1/validate`: check a request without running it. Errors are `{code, message, hint}`.

## Desktop app (Tauri)

`src-tauri/` wraps the same web UI and runs queries **in-process** with the same `playground-core` as the hosted API, against a local MongoDB (default `mongodb://localhost:27017/?directConnection=true`; change it in the app's header, saved in the app config folder). Share links, sample data and stats need the hosted platform and are off. `atlas local setup` gives you a local MongoDB with Atlas Search.

```bash
cd web && npm install && npm run tauri dev      # dev window with hot reload
cd web && npm run tauri build                   # installer (add icons first: npm run tauri icon <1024px png>)
```

## The site

A framed website with a left nav: **Playground** (Data, Shell and Results panels, Run, Explain, Share, **Export**), **REST API**, **MCP server**, **Run it yourself**, **About** (it runs on a MongoDB Atlas M10 cluster), and a footer plus header and sidebar links to create a free Atlas cluster (with UTM parameters). MongoDB-style Evergreen / Spring Green theme with light, dark and auto modes; phones get tabs and a drawer. Pages carry SEO metadata (title, description, keywords, canonical, Open Graph, JSON-LD, `robots.txt`, `sitemap.xml`).

- **Export:** a `mongosh` script that recreates the data, indexes and statements against any MongoDB (`mongosh <uri> --file playground.js`), the data as Extended JSON, the results as JSON or CSV, the transcript as text, or the request as `curl`. API: `POST /api/v1/export`; MCP: `export_playground`.
- **Sample data is a feature flag, off by default** (`FEATURE_SAMPLE_DATA=1` turns it on); the UI hides it and the API rejects `datasource` with a clear error.
- **Abuse controls:** a per-client token bucket on endpoints that do work (`429` with `Retry-After`; env `RATE_BASIC_BURST`, `RATE_BASIC_PER_SEC`, `RATE_VERIFIED_BURST`, `RATE_VERIFIED_PER_SEC`), a higher tier for browsers that pass Cloudflare Turnstile (`TURNSTILE_SITE_KEY`, `TURNSTILE_SECRET`, `SESSION_SECRET`), and a 64 MB ceiling on one statement's results (no cap on the *number* of results).

## Tests

| What | Command |
|---|---|
| Rust unit tests | `cargo test` |
| Rust + HTTP smoke test on a throwaway Atlas local cluster | `scripts/test-with-atlas-local.sh` |
| Frontend unit tests | `cd web && npm test` |
| Browser tests (Playwright: playground, nav, mobile, SEO, agents) | `cd web && npx playwright install chromium` once, then `scripts/e2e.sh` |
| Load test, then adversarial (abuse) test, then a leftover-database check | `scripts/loadtest-with-atlas-local.sh [seconds] [workers]` |

## Run it yourself, get a cluster, connect an agent

[docs/run-it-yourself.md](docs/run-it-yourself.md) (also in the app under **Run it yourself**, at `GET /api/v1/guide`, and as the MCP tool `get_setup_guide`) covers: the hosted site, an agent over MCP, a local deployment with the Atlas CLI (no account; includes Atlas Search), a **free-forever Atlas M0 cluster** (sign-up link with UTM parameters), an **ephemeral Atlas cluster** (create, use, delete), Docker Compose, and the desktop app.

**MCP:** `claude mcp add --transport http mongodb-playground <site>/mcp` (discovery card at `/.well-known/mcp.json`; tools: `run_query`, `validate_query`, `explain_query`, `share_query`, `get_shared`, `list_examples`, `get_setup_guide`, `get_limits`). Everything the web app does is also an API call; the **API & MCP** dialog shows the current script as `curl`.

## Deploying

`infra/` is the Terraform for GCP, Atlas (M10 over Private Service Connect) and Cloudflare; see [infra/README.md](infra/README.md). `cloudbuild.yaml` tests, builds and deploys. `Dockerfile.api` builds one image: the API plus the web app. `docker compose up` runs a local stack (Atlas Local image, so search works).

## Share links, sample data, search index, stats

- **Share:** `POST /api/v1/share` (same body as `run`) returns `{id, url}`; `GET /api/v1/p/{id}` loads it (the web app opens `/p/<id>`). Each share gets a random UUID link (unguessable), 90-day TTL refreshed on view, 4 MB limit.
- **Sample data:** `"datasource": {"sample": "sample_mflix"}` queries an Atlas sample database read-only (cap 1000 documents). Load the sample data with `atlas clusters sampleData load <cluster>` (or the local deployment's sample-data option), and use a read-only user via `MONGODB_SAMPLE_URI`.
- **Search index:** `"searchIndexes": {"movies": {"definition": {"mappings": {"dynamic": true}}}}` (one per collection) builds an Atlas Search index before the query. Needs Atlas or `atlas local`; plain `mongod` is rejected with a clear message.
- **Stats:** every interaction writes a small metadata-only event (no query text, no raw IPs) to the `playground-platform` database. `STATS_DISABLED=1` turns it off.
- **Config (env):** `MONGODB_URI`, `MONGODB_SAMPLE_URI`, `MONGODB_SHARES_URI`, `STATS_MONGODB_URI`, `SHARES_DB`, `STATS_DB`, `STATS_SALT`, `APP_ENV`, `INTERNAL_TOKEN` (enables `POST /internal/jobs/sweep` and `/internal/jobs/rollup`), `PORT`, `LOG_JSON`.

## The shell

Write mongosh-style statements, several in a row; they run in order against one throwaway database, and the result is a transcript.

```js
db.people.insertMany([{ name: "Tim", age: 30 }, { name: "Bo", age: 41 }])
db.people.updateMany({ age: { $gt: 35 } }, { $set: { senior: true } })
db.people.find({ senior: true }, { _id: 0, name: 1 }).sort({ name: 1 })
db.people.createIndex({ age: 1 })
db.people.find({ age: 30 }).explain()
```

Supported: `find`/`findOne` (+ `.sort .limit .skip .projection .count .explain`), `aggregate`, `countDocuments`, `estimatedDocumentCount`, `distinct`, `insertOne/Many`, `updateOne/Many`, `replaceOne`, `deleteOne/Many`, `createIndex`, `dropIndex`, `getIndexes`, `createSearchIndex`. Values are shell syntax or JSON: unquoted keys, single quotes, `/regex/i`, `ObjectId()`, `ISODate()`, `NumberLong()`, comments. `$where`, `$function`, `$out`, `$merge` and cross-database references are rejected.

**The data panel** holds everything about your collections in one place. Three shapes, from a bare array to full per-collection setup:

```js
[ { name: "Ada" }, { name: "Linus" } ]                  // single collection, queried as db.collection

db = { people: [ ... ], orders: [ ... ] }               // multiple collections

db = { people: {                                        // per collection: data, indexes, search, vector search
  documents: [ ... ],
  indexes: [ { key: { age: 1 } } ],                                           // up to 3
  searchIndexes: [ { definition: { mappings: { dynamic: true } } } ],         // Atlas Search, 1
  vectorSearchIndexes: [ { definition: { fields: [ { type: "vector", path: "embedding", numDimensions: 3, similarity: "cosine" } ] } } ]  // 1
} }
```

Limits: 8 collections, 256 documents each, 25 statements, 32 MB payload, 10 s per statement, no cap on results. Search runs need Atlas or `atlas local`.

**API:** `POST /api/v1/run` with `{"script": "...", "config": "..."}`; add `"explain": "executionStats"` or call `/api/v1/explain` for plans. A failing statement stops the script (`ok: false`, earlier output kept). The older single-operation form (`mode`/`collection`/`query`) still works; see `/llms.txt`.

## Not verified yet

- The Turnstile flow against real Cloudflare, and anything behind Cloudflare, Cloud Armor or Cloud Run.
- The Terraform is being applied for the first time; expect provider-schema, Atlas-privilege and PSC-timing fixes. See [infra/README.md](infra/README.md).
- The Tauri app compiles but has not been launched.
