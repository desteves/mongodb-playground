# MongoDB Playground

**An online MongoDB playground for builders: humans and AI agents alike.** Paste your own documents, run mongosh-style
statements (CRUD, aggregation, indexes, Atlas Search, Vector Search), read the query plan, and share the result
with a link. No account, no cluster to set up. It runs on a real MongoDB Atlas cluster, and each run uses a
throwaway database that is deleted afterwards. Live at **https://mongoplayground.com**.

Everything a person can click, an agent can call: a versioned REST API (`/api/v1`), an OpenAPI 3.1 spec,
`llms.txt`, an MCP server, and a Markdown version of every page. See [AGENTS.md](AGENTS.md) if you are an agent (or are
pointing one at this repo).

| You are | Start here |
|---|---|
| A person who wants to try a query | https://mongoplayground.com (Data, Shell and Results panels), or [Run it locally](#run-it-locally) |
| An agent or tool author | [AGENTS.md](AGENTS.md), [`llms.txt`](llms.txt), `GET /api/v1/openapi.json`, `POST /mcp`, [Markdown for agents](#for-agents) |
| Someone self-hosting or contributing | [CONTRIBUTING.md](CONTRIBUTING.md), [docs/run-it-yourself.md](docs/run-it-yourself.md), [infra/README.md](infra/README.md) |
| Curious about the design | [docs/MASTER_PLAN.md](docs/MASTER_PLAN.md); open work is in [docs/todo.md](docs/todo.md) |

**What it is:** a Rust API (axum + the MongoDB driver) on Cloud Run, a Svelte 5 UI built with Vite and CodeMirror 6, a
Tauri 2 desktop app that reuses the same Rust core, and Terraform for GCP, MongoDB Atlas (private connectivity over
Private Service Connect) and Cloudflare. Keywords: online MongoDB playground, MongoDB playground, online MongoDB
shell, test MongoDB queries, mongosh in the browser, aggregation pipeline tester, Atlas Search and Vector Search
sandbox, MongoDB MCP server.

**Status:** live. The core, API, web UI, MCP server, share links, explain, stats, Terraform and the Cloud Build
pipeline are deployed. Rust, HTTP smoke, browser, load and abuse tests pass against an Atlas CLI local deployment.
Not done yet: the Cloud Armor origin lock is still in preview (see [docs/todo.md](docs/todo.md)), the Tauri app has
not been launched, and the Turnstile flow has only been exercised against the live site.

## Run it locally

You need Rust (rustup), Node 22+, and a MongoDB.

1. Start a MongoDB. Pick one:
   - Docker: `docker compose up mongo`
   - Atlas CLI local deployment (includes Atlas Search): `atlas local setup playground --port 27017 --force`
   - Atlas free cluster (M0): `atlas setup`, then put its connection string in `MONGODB_URI`
   - An Atlas ephemeral cluster (temporary, expires in 7 days): see [docs/run-it-yourself.md](docs/run-it-yourself.md)
2. API: `MONGODB_URI=mongodb://localhost:27017 cargo run -p playground-api` (listens on :8080)
3. Web: `cd web && npm install && npm run dev` (http://localhost:5173, proxies `/api` to :8080)

Atlas CLI flags change between releases; check `atlas <command> --help` before relying on the commands above.

## The site

A framed website with a left nav: **Playground**, **Quickstart**, **REST API**, **MCP server**, **Run it yourself** and
**About**, with a MongoDB-style Evergreen / Spring Green theme (light, dark and auto; phones get tabs and a drawer).

- **Three panels.** **Data** (your documents, indexes, Atlas Search and Vector Search indexes), **Shell** (mongosh
  statements) and **Results** (a transcript, plans and errors). Each panel has its own shade, a one-line description and
  can be collapsed. **Run all** and **Run selection** (Ctrl/Cmd+Enter runs the highlighted statements, or everything)
  and **Explain plan** live in the Shell panel. A spinner, the elapsed seconds and a progress bar show a run in flight, and
  Search and Vector Search queries warn that the first run is slower while the index builds.
- **Share.** Creates a random UUID link in a dialog (copy or open); `GET /api/v1/p/{id}` loads it.
- **Export.** Always a preview first, with Copy and a Download confirmation: a `mongosh` script that recreates the data,
  indexes and statements (`mongosh <uri> --file playground.js`), the data as Extended JSON, the results as JSON, CSV or
  text, or the request as `curl`. API: `POST /api/v1/export`; MCP: `export_playground`.
- **Sample data is a feature flag, off by default** (`FEATURE_SAMPLE_DATA=1`; the Terraform variable `feature_sample_data` sets it on the deployed service, default on, and an example is only offered when its sample database exists on the cluster); the UI hides it and the API rejects
  `datasource` with a clear error. Templates and examples never depend on it.
- **Links to mongodb.com** (any variant: www, cloud, docs, dochub) carry one fixed set of UTM parameters, defined once in
  `UTM` in `crates/core/src/lib.rs` and applied by `with_utm` (Rust) and `withUtm` (web); static docs carry the literal query.
- **SEO:** per-page titles, descriptions, canonical URLs, Open Graph, JSON-LD, `robots.txt` and `sitemap.xml`.

## For agents

- `GET /llms.txt`: quick start. `GET /api/v1/openapi.json` (also `/openapi.json`): OpenAPI 3.1.
- `POST /mcp` (also `/api/v1/mcp`): MCP server, Streamable HTTP, stateless. Tools: `run_query`, `validate_query`,
  `explain_query`, `share_query`, `get_shared`, `export_playground`, `list_examples`, `get_setup_guide`, `get_limits`.
  Discovery card at `/.well-known/mcp.json`. Claude Code: `claude mcp add --transport http mongodb-playground <site>/mcp`.
- `POST /api/v1/validate`: check a request without running it. Errors are `{code, message, hint}`.
- **Markdown for agents.** Every page is also served as Markdown: send `Accept: text/markdown` to the page URL, or add
  `.md` to the path (`/index.md`, `/docs/quickstart.md`, `/docs/api.md`, `/docs/mcp.md`, `/docs/run-it-yourself.md`,
  `/about.md`). The format follows Cloudflare's [Markdown for Agents](https://developers.cloudflare.com/fundamentals/reference/markdown-for-agents/):
  YAML front matter (`title`, `description`), the body, then the page's JSON-LD in one fenced `json` block, with
  `Content-Type: text/markdown`, `Vary: Accept`, `x-markdown-tokens` and `content-signal` (override with `CONTENT_SIGNAL`).
  The text lives in `crates/api/src/pages/*.md`; the run-it-yourself page is generated from the guide. Browsers still get the app.
- Everything the web app does is also an API call; the Export menu offers the current request as `curl`.

## The shell

Write mongosh-style statements, several in a row; they run in order against one throwaway database, and the result is a transcript.

```js
db.people.insertMany([{ name: "Tim", age: 30 }, { name: "Bo", age: 41 }])
db.people.updateMany({ age: { $gt: 35 } }, { $set: { senior: true } })
db.people.find({ senior: true }, { _id: 0, name: 1 }).sort({ name: 1 })
db.people.createIndex({ age: 1 })
db.people.find({ age: 30 }).explain()
```

Supported: `find`/`findOne` (+ `.sort .limit .skip .projection .count .explain`), `aggregate`, `countDocuments`,
`estimatedDocumentCount`, `distinct`, `insertOne/Many`, `updateOne/Many`, `replaceOne`, `deleteOne/Many`, `createIndex`,
`dropIndex`, `getIndexes`, `createSearchIndex`. Values are shell syntax or JSON: unquoted keys, single quotes, `/regex/i`,
`ObjectId()`, `ISODate()`, `NumberLong()`, comments. `$where`, `$function`, `$out`, `$merge` and cross-database
references are rejected.

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

Limits: 8 collections, 256 documents each, 25 statements, 32 MB payload, 10 s per statement, no cap on the number of
results (one statement's results are bounded at 64 MB). Search runs need Atlas or `atlas local`.

**API:** `POST /api/v1/run` with `{"script": "...", "config": "..."}`; add `"explain": "executionStats"` or call
`/api/v1/explain` for plans. A failing statement stops the script (`ok: false`, earlier output kept). The older
single-operation form (`mode`/`collection`/`query`) still works; see `/llms.txt`.

## Share links, sample data, search index, stats

- **Share:** `POST /api/v1/share` (same body as `run`) returns `{id, url}`. Each share gets a random UUID link
  (unguessable), a 90-day TTL refreshed on view, and a 4 MB limit.
- **Sample data:** `"datasource": {"sample": "sample_mflix"}` queries an Atlas sample database read-only (cap 1000
  documents). Load it with `atlas clusters sampleData load <cluster>`; use a read-only user via `MONGODB_SAMPLE_URI`.
- **Prebuilt example datasets:** `api seed-examples` (with `MONGODB_ADMIN_URI`, a user that can write and create search
  indexes) builds `sample_playground`: `movies` with an Atlas Search index and `items` with a Vector Search index, so the
  two "prebuilt" examples answer instantly instead of waiting for an index build. They are read through the sample-data
  path, so they appear when `FEATURE_SAMPLE_DATA=1` and the sample user can read `sample_playground`
  (the Terraform default does). The inline examples keep working without any of it.
- **Search index:** `"searchIndexes"` / `"vectorSearchIndexes"` (one of each per collection) build an Atlas Search or
  Vector Search index before the query. Needs Atlas or `atlas local`; plain `mongod` is rejected with a clear message.
- **Stats:** every interaction writes a small metadata-only event (no query text, no documents, no raw IPs) to the
  `playground-platform` database. `STATS_DISABLED=1` turns it off.
- **Share limits:** 30 new share links per client address per UTC day (`SHARES_PER_CLIENT_PER_DAY`; `429` with
  `Retry-After`, counted per instance) and a total cap (`SHARES_MAX_TOTAL`, default 100000; `503` with code `capacity`).
  `DELETE /internal/shares/<id>` (internal token) takes a share down, and `/internal/jobs/rollup` reports usage and logs a
  warning above 80 % of the cap.
- **Abuse controls:** a per-client token bucket on endpoints that do work (`429` with `Retry-After`), a higher tier for
  browsers that pass Cloudflare Turnstile, and Cloudflare rate limits and Cloud Armor in front.
- **Config (env):** `MONGODB_URI`, `MONGODB_SAMPLE_URI`, `MONGODB_SHARES_URI`, `STATS_MONGODB_URI`, `SHARES_DB`,
  `STATS_DB`, `STATS_SALT`, `STATS_DISABLED`, `PUBLIC_URL` (builds the links in the Markdown pages and `llms.txt`),
  `CLUSTER_TIER` (shown on the About page), `CONTENT_SIGNAL`, `FEATURE_SAMPLE_DATA`, `WEB_DIR`, `APP_ENV`,
  `INTERNAL_TOKEN` (enables `POST /internal/jobs/sweep`, `/internal/jobs/rollup` and `DELETE /internal/shares/<id>`), `SHARES_MAX_TOTAL`, `SHARES_PER_CLIENT_PER_DAY`, `MONGODB_ADMIN_URI` (only for `api seed-examples`), `PORT`, `LOG_JSON`,
  `RATE_BASIC_BURST`, `RATE_BASIC_PER_SEC`, `RATE_VERIFIED_BURST`, `RATE_VERIFIED_PER_SEC`, `TURNSTILE_SITE_KEY`,
  `TURNSTILE_SECRET`, `SESSION_SECRET`.

## Caching

The API sets `Cache-Control` itself and Cloudflare follows it (rules in `infra/modules/cloudflare`). What is cached, and
what never is:

| What | Cached | Why |
|---|---|---|
| `/assets/*` (hashed build output) | 1 year, edge and browser, `immutable` | A deploy changes the file names |
| `/favicon.svg`, `/robots.txt`, `/sitemap.xml` | 1 hour | Change only on deploy |
| `*.md` page twins (`/about.md`, ...) | 1 hour edge, 5 minutes browser | One URL, one representation |
| `/llms.txt`, `/openapi.json`, `/api/v1/{openapi.json,examples,guide}`, `/.well-known/mcp.json` | 5 minutes | Change only on deploy; keeps agent polling off the app |
| `/api/v1/limits` | 1 minute | Calls the database (search support) |
| `/api/v1/p/{id}` (shares) | at most 1 day | Content is immutable, but shares expire after 90 days without a view |
| HTML pages (`/`, `/about`, `/docs/*`, `/p/<id>`) | **never** at the edge; the browser revalidates (`no-cache`) | The same URL returns HTML or Markdown depending on `Accept`, and a shared cache without `Accept` in its key would serve the wrong one |
| `POST /api/v1/*`, `/mcp`, `/readyz`, `/internal/*` | **never** | Work, per-client or live state |

## Tests

| What | Command |
|---|---|
| Rust unit tests | `cargo test` |
| Rust, integration and an HTTP smoke test on a throwaway Atlas local deployment (port 6310) | `scripts/test-with-atlas-local.sh` |
| Frontend unit tests | `cd web && npm test` |
| Browser tests (Playwright: playground, nav, mobile, SEO, agents, Markdown) | `cd web && npx playwright install chromium` once, then `scripts/e2e.sh` |
| Load test, then adversarial (abuse) test, then a leftover-database check | `scripts/loadtest-with-atlas-local.sh [seconds] [workers]` |
| Smoke-test an already running API | `python3 scripts/smoke.py [base-url]` |
| Cloud Armor decisions, before enforcing the origin lock | `python3 scripts/armor-report.py [hours]`, `python3 scripts/check-cloudflare-ips.py` |
| Regenerate `docs/run-it-yourself.md` from the guide | `python3 scripts/gen-run-it-yourself.py` |

## Run it yourself, get a cluster, connect an agent

[docs/run-it-yourself.md](docs/run-it-yourself.md) (also in the app under **Run it yourself**, at `GET /api/v1/guide`, as
Markdown at `/docs/run-it-yourself.md`, and as the MCP tool `get_setup_guide`) starts with a chooser and covers: the hosted
site, an agent over MCP, a local deployment with the Atlas CLI (no account; includes Atlas Search), Docker Compose, the
desktop app, a **free-forever Atlas M0 cluster** (persistent, in your account), and an **ephemeral Atlas cluster** (a
temporary free cluster that Atlas pauses after 2 days and deletes after 7 unless you claim it).

## Desktop app (Tauri)

`src-tauri/` wraps the same web UI and runs queries **in-process** with the same `playground-core` as the hosted API,
against a local MongoDB (default `mongodb://localhost:27017/?directConnection=true`; change it in the app's header, saved
in the app config folder). Share links, sample data and stats need the hosted platform and are off.

```bash
cd web && npm install && npm run tauri dev      # dev window with hot reload
cd web && npm run tauri build                   # installer (add icons first: npm run tauri icon <1024px png>)
```

## Deploying

`infra/` is the Terraform for GCP (Cloud Run, load balancer, Cloud Armor, Secret Manager, Cloud Build, monitoring), Atlas
(the cluster, a port-mapped Private Service Connect endpoint, one database user per job) and Cloudflare (DNS, TLS, an
origin-key header, rate limits, cache rules, Turnstile); see [infra/README.md](infra/README.md). `cloudbuild.yaml` tests,
builds and deploys on every push to `main`; `Dockerfile.api` builds one image (the API plus the web app). `docker compose up`
runs a local stack (Atlas Local image, so search works).

Security notes for contributors:
- **No database credential is ever stored in Terraform state or a plan file.** The passwords are ephemeral and write-only;
  rotate with `credential_version`. Needs Terraform 1.11 or later.
- **Plans stay out of the repo.** Use `scripts/tf.sh plan` and `scripts/tf.sh apply` (they keep the plan outside the
  checkout), and run `scripts/install-hooks.sh` once: the pre-commit hook blocks plan, state, `.env` and `*.tfvars` files and
  anything that looks like a credential.
- Atlas is reachable only through the private endpoint; the load balancer accepts only Cloudflare traffic (the origin lock is
  being rolled out, see [docs/todo.md](docs/todo.md)); the app rejects `$where`, `$function`, `$out`, `$merge` and cross-database references.

## Not verified yet

- The Cloud Armor origin lock in enforcing mode, and the Turnstile flow beyond the live site.
- The Tauri app compiles but has not been launched.
