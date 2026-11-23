# MongoDB Playground: design and architecture

An online MongoDB playground for builders, human and agent alike. You supply documents, run mongosh-style
statements, read the query plan, and share the result with a link. This document describes how it is built
and why. For day-to-day use see the [README](../README.md); for agents see [AGENTS.md](../AGENTS.md).

## Goals

- **Try a query in seconds.** No account, no cluster to set up, real MongoDB behavior.
- **A real shell feel.** Several `db.collection.method()` statements per run, in order, with a transcript.
- **Understand performance.** Query plans (`explain`) with plain-language advice, indexes, Atlas Search and Vector Search.
- **Agents are first-class.** Everything the UI does is available through a versioned REST API, an OpenAPI
  document, `llms.txt` and an MCP server.
- **Easy to run yourself.** Locally, on a free Atlas cluster, or on an ephemeral one.

Non-goals: connecting to a user's own production cluster, server-side JavaScript, long-lived data storage.

## Architecture

```
Browser (Svelte SPA) ─┐
Tauri desktop app ────┤   Cloudflare (DNS, proxy, WAF, rate limits, cache)
                      └─> Global HTTPS load balancer + Cloud Armor
                            └─> Cloud Run: Rust API (axum) that also serves the SPA
                                  └─ Direct VPC egress ─> Private Service Connect ─> Atlas M10
                                                                ├─ pg_<id>            throwaway run databases
                                                                ├─ sample_*           optional, read-only
                                                                ├─ shares             share links
                                                                └─ playground-platform  usage stats
```

- **API:** Rust (`axum`, `tokio`, the official `mongodb` driver) on Cloud Run. Cloud Run functions have no
  first-class Rust runtime, so the API ships as a container with scale-to-zero billing.
- **UI:** Svelte 5 with CodeMirror 6, built by Vite into static files that the API container serves. The
  Tauri 2 desktop app wraps the same build.
- **Rust workspace:** `crates/core` (parser, validation, runner, explain, share, export, stats),
  `crates/api` (HTTP, rate limiting, MCP) and `src-tauri` (desktop app, runs `core` in-process).
- **Database:** an Atlas M10 reached only over Private Service Connect. No public path.
- **Edge:** Cloudflare proxies the domain; the load balancer accepts only Cloudflare traffic.
- **Infrastructure and CI:** Terraform for GCP, Atlas and Cloudflare; Cloud Build for test, build and deploy.

### Execution model

Every run uses a throwaway database on a shared cluster: validate the request, create `pg_<time>_<random>`,
seed the documents and indexes, run the statements in order, collect the output, and always drop the database
(with a sweep job as a backstop). Alternatives considered:

| Approach | Trade-off | Verdict |
|---|---|---|
| Shared cluster, throwaway database per run | Real MongoDB semantics, cheap, simple; needs strict validation | **Chosen** |
| Fresh MongoDB container per run | Best isolation; seconds of cold start and higher cost | Rejected for now |
| Emulate MongoDB in Rust | Fast; semantic drift makes a playground misleading | Rejected |

## The shell and the data panel

Statements are mongosh syntax, run in order against one database; a failing statement stops the script and
keeps earlier output (`ok: false`). Supported: `find`/`findOne` (with `.sort .limit .skip .projection .count
.explain`), `aggregate`, `countDocuments`, `estimatedDocumentCount`, `distinct`, `insertOne/Many`,
`updateOne/Many`, `replaceOne`, `deleteOne/Many`, `createIndex`, `dropIndex`, `getIndexes`, `createSearchIndex`.

Values may be shell syntax or JSON: unquoted keys, single quotes, comments, trailing commas, `/regex/i`
literals, `ObjectId()`, `ISODate()`, `NumberLong()`. A hand-written parser turns them into Extended JSON, so
there is **no JavaScript evaluation** anywhere.

The **Data panel** holds everything about your collections in one place, in three shapes: a bare array (one
collection, queried as `db.collection`), `db = { name: [...] }` for several collections, and
`db = { name: { documents, indexes, searchIndexes, vectorSearchIndexes } }` for per-collection setup.

### Limits

| Limit | Value |
|---|---|
| Collections per run | 8 |
| Documents per collection | 256 |
| Statements per run | 25 |
| Payload (uncompressed) | 32 MB |
| Time per statement (`maxTimeMS`) | 10 s |
| Indexes per collection | 3 (up to 4 key fields each) |
| Atlas Search / Vector Search indexes per collection | 1 each |
| Regex literals per request / pattern length | 20 / 256 chars |
| Number of results | not capped |
| Result bytes for one statement | 64 MB ceiling |

Because the number of results is not capped, runs are bounded by the query time limit, the 64 MB result-byte
ceiling, and the input size. Aggregations that multiply data are the main risk, so the runner drains cursors
with `getMore` on one explicit session and aborts past the ceiling.

### Rejected operators

`$where`, `$function`, `$accumulator`, `$out`, `$merge`, cross-database `$lookup`/`$unionWith`/`$graphLookup`,
and admin stages such as `$currentOp`, `$listSessions` and `$collStats`. The server validates every request
against this allowlist before touching the database, and every write targets only the run's own database.

## Explain plans

Any read or write can be explained (`POST /api/v1/explain`, or `explain` on `/run` for results plus the plan,
or `.explain()` in a statement). Verbosity is `queryPlanner`, `executionStats` (default) or
`allPlansExecution`. The response has the raw server plan and a summary for humans and agents: stage tree,
`indexUsed`, `collscan`, keys and documents examined, documents returned, timing, and plain-language `advice`
(for example "COLLSCAN: no index supports this filter; try an index on {age: 1}"). Explaining a write does not
perform it.

## Search and Vector Search

A collection can declare one Atlas Search index and one Vector Search index. The runner creates them, waits
until they are queryable, then runs the statements. Because builds take seconds to a minute, search runs get a
longer budget (60 s) and share a global concurrency cap so the cluster is not starved. Search needs Atlas or
an Atlas CLI local deployment; plain `mongod` is refused with a clear message. `GET /api/v1/limits` reports
`searchSupported`.

## Sample data (feature flag, off by default)

With `FEATURE_SAMPLE_DATA=1`, scripts can query an Atlas sample database read-only through a separate
database user (reads only, capped at 1000 documents). It is off by default: the UI hides it and the API
rejects `datasource` with a clear error. Templates never depend on it.

## Share links

`POST /api/v1/share` stores the validated playground and returns a random UUID link (`/p/<uuid>`), so links
cannot be guessed. They expire 90 days after their last view and are limited to 4 MB. A content hash is stored
alongside for analytics. The share store uses a different database and user from the run databases, and the UI
warns people not to share sensitive data.

## API

Everything is under `/api/v1`: `run`, `validate`, `explain`, `share`, `p/{id}`, `export`, `examples`, `guide`,
`limits`, `samples`, `events`, `verify`, `openapi.json`, `mcp`. Discovery aliases live at `/llms.txt`,
`/openapi.json`, `/mcp` and `/.well-known/mcp.json`; `/healthz` and `/readyz` are unversioned. A breaking
change would ship as `/api/v2` next to v1. Errors are `{code, message, hint}` with stable codes.

- **HTTP compression:** gzip, brotli, zstd and deflate request bodies are accepted and responses are
  compressed. The 32 MB limit applies to the decompressed body, which also defuses decompression bombs.
- **Wire compression:** zstd, snappy and zlib are enabled in the driver unless the URI says otherwise.
- **Connection practices** (a serverless profile; each applies only if the URI leaves it unset):

| Setting | Value | Why |
|---|---|---|
| One shared client | created at startup | Reused across requests, never per request |
| `maxPoolSize` | 5 | One connection per in-flight run; matches Cloud Run per-instance concurrency |
| `minPoolSize` | 0 | Scale-to-zero instances should not hold idle server slots |
| `maxIdleTimeMS` | 30000 | Release unused connections quickly |
| `connectTimeoutMS` / `serverSelectionTimeoutMS` | 10 s / 5 s | Fail fast |
| `retryReads` / `retryWrites` | true | Explicit |
| `appName` | `mongodb-playground` | Identifies the app in Atlas logs and metrics |
| Stable API | `v1`, not strict | Stable behavior across server upgrades while still allowing any operator |
| `mongodb+srv://` private-endpoint string | production | TLS on by default; private path only |
| Credentials | Secret Manager | One URI per database user; never in the repo or the image |

Pool size, Cloud Run concurrency and instance count are starting points to revisit with real traffic.

## Agents are first-class

An agent can discover the service and run queries with no browser and no human.

| Need | How it is met |
|---|---|
| Discovery | `/llms.txt`, OpenAPI 3.1, `/.well-known/mcp.json` |
| Tool use | Stateless MCP over Streamable HTTP with nine tools: `run_query`, `validate_query`, `explain_query`, `share_query`, `get_shared`, `export_playground`, `list_examples`, `get_setup_guide`, `get_limits` |
| Safe iteration | `validate` checks a request without a database; every example is a valid request body (a test asserts it) |
| Self-correcting errors | Every error carries a `hint` |
| No browser-only gates | Turnstile raises a browser's rate-limit tier but is never required; agents use the basic tier |
| Parity | Every UI feature has an endpoint, an OpenAPI entry and, where useful, an MCP tool |

## Usage statistics

Every interaction produces one small event, written fire-and-forget through a bounded queue that never blocks
a run. Events go to the `playground-platform` database (a time-series `events` collection with a 13-month TTL,
plus daily roll-ups). They hold metadata and hashes only: **no query text, no documents, no raw IP addresses**.
IP hashes are salted with a daily-rotating salt, session IDs are random, and `STATS_DISABLED=1` turns the
whole thing off. A separate database user can write only to this database.

## Security model

1. **Parse, don't evaluate.** No server-side JavaScript. An operator allowlist, nesting depth and size limits.
2. **Namespace pinning.** The runner only ever touches its own `pg_*` database; cross-database references are
   rejected in the parser. Separate database users for runs, shares, stats and sample data.
3. **Resource bounds.** `maxTimeMS`, result-byte ceiling, document and payload caps, Cloud Run concurrency and
   instance limits, a search-run concurrency cap, and a janitor that drops leftover databases.
4. **Rate limiting.** A per-client token bucket on endpoints that do work (`429` with `Retry-After` and
   `X-RateLimit-*` headers), a higher tier for browsers that pass Cloudflare Turnstile, and Cloudflare and
   Cloud Armor rules in front.
5. **Origin lock.** Cloudflare sets a secret `X-Origin-Key` header that clients cannot supply, plus an IP
   allowlist of Cloudflare ranges. Cloud Armor starts in preview mode (`enforce_origin_restriction`) so a wrong
   rule cannot take the site down.
6. **WAF exceptions.** Query bodies look like attacks to generic XSS and SQL-injection rules, so
   `/api/v1/run`, `/explain` and `/share` are excluded from them; the application-side allowlist is the real
   control.
7. **Private data path.** Atlas is reachable only over Private Service Connect; secrets live in Secret
   Manager; Cloud Build runs as a scoped service account.

## Infrastructure and delivery

`infra/` is flat Terraform composed of modules: `bootstrap`, `iam`, `network`, `atlas`, `secrets`, `runtime`,
`edge`, `cloudflare`, `cicd` and `observability`.

- **GCP:** Artifact Registry, Cloud Run with a least-privilege service account and Direct VPC egress, Secret
  Manager, a global HTTPS load balancer with a managed certificate and a Cloud Armor policy, Cloud Scheduler
  jobs for the sweep and roll-up, budget and log-based alerts, and Cloud Build triggers.
- **Atlas:** an M10 cluster, Private Service Connect, and one database user per job.
- **Cloudflare:** proxied DNS records, strict TLS, an origin-key transform rule, rate-limit and cache rules,
  and a Turnstile widget.
- **Cloud Build** (`cloudbuild.yaml`): lint and tests first, then build the image, push it, and deploy that
  exact SHA to Cloud Run. Tauri installers are built locally, since Cloud Build has no macOS or Windows workers.

Credentials come from environment variables or Secret Manager, never from the repository. See
[infra/README.md](../infra/README.md) for the deploy steps.

## Run it yourself

All paths use the Atlas CLI and are described in [run-it-yourself.md](run-it-yourself.md), in the app under
**Run it yourself**, at `GET /api/v1/guide`, and through the MCP tool `get_setup_guide`:

1. **Local, no account:** an Atlas CLI local deployment (includes Atlas Search), then `cargo run -p playground-api`.
2. **Free-forever Atlas M0:** sign up, create the cluster, a database user and an access-list entry.
3. **Ephemeral cluster:** a temporary free Atlas cluster created with the Atlas CLI's `atlas api ephemeralClusters createEphemeralCluster` (a preview API). Atlas pauses it after 2 days and deletes it after 7 unless it is claimed into an account, so there is nothing to clean up; a local deployment is the zero-cost option.
4. **Docker Compose:** a local stack with the Atlas Local image so search works.
5. **Desktop app:** the Tauri app against a local MongoDB.

## Testing

Rust unit tests; integration and HTTP smoke tests against a throwaway Atlas CLI local deployment on port 6310
(`scripts/test-with-atlas-local.sh`); frontend unit tests (vitest); browser tests (Playwright, desktop and
mobile projects); a load test followed by an adversarial test and a leftover-database check
(`scripts/loadtest-with-atlas-local.sh`).

## Risks and open items

- **Search on the M10.** There is no hard cap on search indexes at M10 and above, but many indexes on a small
  tier add load and memory pressure. The practical limits are one per collection, eight collections per run,
  and the global search-run concurrency cap.
- **Cross-database protection** rests on parser validation and separate database users, because Atlas custom
  roles cannot wildcard-match `pg_*` names. This needs ongoing tests and fuzzing.
- **Abuse** (heavy pipelines, storage spam): caps, rate limits, TTLs, a budget alert and the sweep job.
- **Fixed costs:** the M10 and the load balancer run all the time.
- **Not yet exercised against real accounts:** Turnstile through real Cloudflare, behavior behind Cloud Armor
  and Cloud Run, and the Tauri app launch.
