# To do

Open work, newest decisions first. Cross items off (or delete them) when they ship.

## 0. Open right now (as of 2026-11-23)

- [ ] **Cloud Armor stage 2** (`enforce_origin_restriction = true`): after a few clean days of stage 1, re-run
  `python3 scripts/armor-report.py 24`, then follow section 1.
- [ ] **Prebuilt example datasets are not confirmed live.** `FEATURE_SAMPLE_DATA` is on, but check that
  `curl -s https://mongoplayground.com/api/v1/examples | grep -o '"id": *"prebuilt-[a-z-]*"'` lists both
  `prebuilt-atlas-search` and `prebuilt-vector-search`. If not, `sample_playground` was never seeded: create a temporary
  Atlas admin user, run `MONGODB_ADMIN_URI='<uri>' cargo run -p playground-api -- seed-examples`, wait up to ten
  minutes (the sample database list is cached), re-check, then delete the temporary user.
- [ ] **Verify the alert email channel.** Google sent a verification email to the `notification_email` address for the
  new channel; click it, or the "Playground is down" and share-storage alerts will not arrive.
- [x] **Soft 404s.** Done: unknown paths now return a real `404` (with the app's HTML, so it still says "Page not found"). The list of real pages is `is_spa_route` in `crates/api/src/main.rs`; add a route there whenever the web app gets a new page (for example `/compass`).
- [ ] **Raise HSTS** from 1 month to 6 or 12 months once nothing needs the site reachable without HTTPS (for example
  re-issuing the origin certificate with the proxy off); see `infra/modules/cloudflare/main.tf`.
- [ ] **Uncommitted local changes:** Cloud Build edits (deploy waits only for the image push, `FUZZ_ITERATIONS`), the
  validator fuzz test, Terraform (HSTS, `/og.png` cache rule, drift fix, two-stage origin lock, alerts), the desktop
  script (`scripts/dev-desktop.sh`), the new `scripts/armor-report.py`, and the README, MASTER_PLAN and todo updates.
- [ ] **Desktop app:** launch it once (`scripts/dev-desktop.sh`); it compiles but has never been opened.
- [x] Done today: Cloud Armor stage 1 is enforcing and verified (direct request `403`, site healthy, no real traffic
  denied, the Cloud Scheduler rollup job still succeeds).

## 1. Turn on the Cloud Armor origin lock (two stages)

**Status:** stage 1 is ON (applied 2026-11-23): requests without our secret `X-Origin-Key` get `403` at the load
balancer. Verified: the direct request to the load balancer IP returns `403` (it took a few minutes to propagate),
`/readyz` through Cloudflare returns `ok`, and `scripts/armor-report.py 3` showed no real traffic missing the key.
Stage 2 (Cloudflare IP ranges, `enforce_origin_restriction`) is still in preview: do it after a few clean days.
Cloud Run's own ingress already accepts only the load balancer, so the lock is a second
layer: it makes the load balancer accept only requests that came through our Cloudflare zone. A wrong rule is an
outage, so it goes in two stages, each a one-line change you can roll back in a minute or two.

- **Stage 1, `enforce_origin_key = true`:** deny any request without our secret `X-Origin-Key` header (Cloudflare adds
  it to every proxied request). Nobody can reach the load balancer directly. This does not depend on Cloudflare's IP
  list, so it cannot lock out visitors over a stale list. It is nearly all of the benefit.
- **Stage 2, `enforce_origin_restriction = true`** (includes stage 1): also deny every source outside Cloudflare's
  published ranges (defence in depth if the key ever leaked). Needs the ranges in Terraform to be current.

**Already checked (2026-11-23):** the 15 IPv4 and 7 IPv6 ranges in `infra/modules/edge/main.tf` match
https://www.cloudflare.com/ips-v4 and `/ips-v6` exactly. Re-check before stage 2 if time has passed:
`python3 scripts/check-cloudflare-ips.py` (it needs network access to cloudflare.com). The other callers of the site
all use the public domain, so they pass through Cloudflare and carry the header: the Cloud Scheduler jobs
(`https://<domain>/internal/jobs/...`) and the uptime check (`/readyz` on the domain).

**Before stage 1** (done)

1. Generate real traffic: open https://mongoplayground.com, run a query, press Share.
2. Read what Armor decided: `python3 scripts/armor-report.py 24`. For real traffic the number of preview denies
   (wrong or missing origin key) must be zero. Your own direct `curl --resolve` tests to the load balancer IP are
   expected to show up; that is what the lock blocks.
3. Raw log query, if you want to look yourself:
   `gcloud logging read 'resource.type="http_load_balancer" AND jsonPayload.previewSecurityPolicy.outcome="DENY"' --project playground-515215 --freshness=24h --limit=20 --format="table(timestamp,httpRequest.remoteIp,httpRequest.requestUrl,httpRequest.status)"`

**Stage 1** (done)

1. `echo 'enforce_origin_key = true' >> infra/terraform.tfvars`
2. `scripts/tf.sh plan`: it should change only the Cloud Armor policy (rule 2100 leaves preview). Stop if anything else
   changes.
3. `scripts/tf.sh apply`
4. Verify: `curl -s https://mongoplayground.com/readyz` returns `ok`, and the direct path is refused:
   `curl -sk --resolve mongoplayground.com:443:136.68.121.105 https://mongoplayground.com/readyz -o /dev/null -w "%{http_code}\n"`
   returns `403`. Also run a query and press Share in the browser, and check the next scheduler run
   (`gcloud scheduler jobs run playground-rollup --location us-central1`) and the uptime check stay green.

**Stage 2** (after stage 1 has run cleanly for a few days)

1. Before this, the default-rule hits must be zero for real traffic:
   `gcloud logging read 'resource.type="http_load_balancer" AND jsonPayload.enforcedSecurityPolicy.priority=2147483647' --project playground-515215 --freshness=24h --limit=20 --format="table(timestamp,httpRequest.remoteIp,httpRequest.requestUrl,httpRequest.status)"`
   (this means a source outside the Cloudflare ranges got through the allow rules).
2. `echo 'enforce_origin_restriction = true' >> infra/terraform.tfvars`, then `scripts/tf.sh plan` (only the default rule
   goes from `allow` to `deny(403)`), then `scripts/tf.sh apply`, then repeat the verification above.

**Roll back (a minute or two):** set the variable you turned on back to `false` in `infra/terraform.tfvars`, then
`scripts/tf.sh plan` and `scripts/tf.sh apply`.

**Rotating the origin key** (`scripts/tf.sh plan -replace=random_password.origin_key`): the Cloudflare rule and the
Armor rule both change in one apply, so expect a few seconds of 403s while they converge. Do it off-peak.

Related housekeeping still open: delete or downgrade the Project Owner Atlas key, rotate every key and token that
was pasted into chats, ask GitHub to purge commit `f44463f1a2e4ca8eeb5d8b75f14a8d37130a1e91`, and enable secret
scanning and push protection on the repository.

## 2. SEO and AEO (answer-engine optimisation)

**Decision to make:** lead with **"Online MongoDB Playground"** instead of "Online MongoDB Shell". Reasoning:

- It matches the product name and the domain (`mongoplayground.com`), so title, brand and URL reinforce each other.
- The "shell / compiler / editor / sandbox" wording is where generic code-runner sites compete (OneCompiler,
  myCompiler, freeonlinecompiler, Embedenv, RunCode, Studyopedia, ...). "Playground" is a narrower field
  (mongoplayground.net, Toolaska, MongoPractice, Humongous) where our differentiators stand out.
- Keep "shell", "mongosh", "compiler", "editor" as **secondary** terms in the body, descriptions and keywords, not
  in the headline.

### Keyword candidates

| Tier | Phrases | Where |
|---|---|---|
| Primary | online MongoDB playground, MongoDB playground | title, H1, first sentence, repo description |
| Secondary (alias terms people type) | MongoDB online compiler, MongoDB online editor, online MongoDB shell, mongosh online, mongo shell online, MongoDB sandbox | description, body copy, keywords, FAQ answers |
| Task intent ("I want to...") | test MongoDB queries online, MongoDB query tester, MongoDB aggregation playground, aggregation pipeline tester, MongoDB explain plan online, practice MongoDB queries, share a MongoDB query | page titles for Quickstart and docs, headings, FAQ |
| Differentiators (little competition) | Atlas Search playground, Vector Search playground, MongoDB MCP server, MongoDB sandbox for AI agents, MongoDB REST API playground, mongosh over HTTP, real MongoDB (not a simulator) | feature sections, About, MCP and API pages, JSON-LD `featureList` |

Our edge over most competitors (several run against in-page simulators, not a real server): runs on a real MongoDB
Atlas cluster, Atlas Search and Vector Search, query plans with advice, share links, REST API, MCP server, free and
no sign-up. Say "real MongoDB" explicitly.

### Question phrasings to answer (AEO)

Write short, self-contained answers (40 to 60 words, the answer first) on the home page or About, marked up as
`FAQPage`:

- Is there a free online MongoDB playground?
- How do I test a MongoDB query online without installing MongoDB?
- How do I run an aggregation pipeline online?
- How do I try Atlas Search or Vector Search without creating a cluster?
- Can an AI agent run MongoDB queries? (MCP server and REST API)
- How do I share a MongoDB query with someone?
- Is it a real MongoDB or a simulator?

### Where the current wording lives

`web/index.html` (title, description, keywords, OG and Twitter titles, JSON-LD, the no-JavaScript H1 and list),
`web/src/lib/seo.ts` (home, Quickstart, About), `web/src/pages/About.svelte` (lead), `README.md`, `llms.txt`,
`crates/api/src/llms.txt`, `crates/api/src/openapi.json` (description), `web/package.json` and
`crates/api/Cargo.toml` (descriptions), `AGENTS.md`, `docs/MASTER_PLAN.md`, `docs/RELEASE_v0.1.0.md`, the GitHub
repository description and topics, and three assertions in `web/e2e/site.spec.ts` (title and keywords).

### Draft copy (to review)

- Title: `Online MongoDB Playground: Test Queries, Aggregations and Atlas Search | MongoDB Playground`
- Description: `Free online MongoDB playground on a real MongoDB cluster. Paste documents, run mongosh-style find, aggregate, update and delete, see query plans, try Atlas Search and Vector Search, share a link. No sign-up.`
- H1: `Online MongoDB Playground`, with the line "Test MongoDB queries, aggregation pipelines, Atlas Search and Vector Search in your browser, on a real MongoDB cluster."

### Tasks

- [x] Rename the headline wording everywhere listed above; update the e2e assertions. (Done: "Online MongoDB Playground"; the GitHub repository description still needs updating with `gh repo edit`.)
- [x] **Markdown for agents** (done): every page is served as Markdown from the API, with `Accept: text/markdown`
  or a `.md` suffix (`/index.md`, `/about.md`, `/docs/api.md`, ...), plus `Vary: Accept`, a `Link` header to the
  twin, `x-markdown-tokens` and `content-signal`. Source: `crates/api/src/pages.rs` and `crates/api/src/pages/*.md`.
  Covers agents; it does **not** help crawlers that fetch plain HTML (see the next item).
- [x] **Prerender per-route head and body text for HTML.** Done in `crates/api/src/prerender.rs` (title, description, canonical, JSON-LD and the page text in the HTML). Original note: The site is a single-page app that sets
  titles in the browser. Google runs JavaScript, but most AI crawlers do not. Serve route-specific `<title>`,
  description, canonical and a real text body in the HTML for `/`, `/docs/quickstart`, `/docs/api`, `/docs/mcp`,
  `/docs/run-it-yourself` and `/about`. The Markdown pages already hold the text; the HTML could be generated from them.
- [ ] Optional: serve shared playgrounds (`/p/<id>`) as Markdown too (script, data, and the run command).
- [ ] Optional: turn on Cloudflare's own "Markdown for Agents" (Pro plan and up). Not needed now, since the origin
  already answers `Accept: text/markdown` itself; Cloudflare only converts HTML responses, so the two do not clash.
- [x] (done: FAQPage on the home page, HowTo on the Quickstart, in the HTML and the Markdown) Add a FAQ with `FAQPage` JSON-LD (questions above), `HowTo` JSON-LD on the Quickstart, and extend the
  existing `WebApplication` JSON-LD (`isAccessibleForFree`, `sameAs` the GitHub repository, `softwareVersion`).
- [x] (done: `web/public/og.svg` renders to `og.png` with `scripts/make-og-image.sh`; the meta tags are in `index.html`) Open Graph image (1200 x 630) and `twitter:card` = `summary_large_image`.
- [x] (done) `robots.txt`: explicitly allow AI crawlers (GPTBot, ClaudeBot, PerplexityBot, Google-Extended) if wanted;
  keep `sitemap.xml` current with `lastmod`; add `llms-full.txt` next to `llms.txt`.
- [x] Performance (Core Web Vitals): CodeMirror is its own cached chunk, app code 117 kB (see section 4).
- [x] (done) Internal links: link the Quickstart from the playground and the examples from the docs.
- [ ] Search Console and Bing Webmaster Tools: verify the domain, submit the sitemap, watch the queries.
- [ ] Off-site: awesome-mongodb lists, Show HN, dev.to, the MongoDB community forum, and a fair
  "alternatives" page. Keep the GitHub repository description and topics aligned with the new wording.
- [ ] Measure: record rankings for the primary and differentiator phrases monthly, and ask ChatGPT, Perplexity and
  Claude the FAQ questions to see whether the site is cited.

## 3. Caching (decided and implemented; notes for later)

The API sets `Cache-Control` and the Cloudflare cache rules (`infra/modules/cloudflare`) follow it; the full table is
in the README ("Caching"). HTML pages are never cached at the edge because one URL returns HTML or Markdown depending
on `Accept`. If a TTL on a deploy-dependent file is ever raised above an hour, add a purge-on-deploy step to
`cloudbuild.yaml` (needs a Cloudflare token with Cache Purge). Smart Tiered Cache is a dashboard toggle (Caching >
Tiered Cache); not managed in Terraform.

## 4. Security and performance backlog

Ordered roughly by value for effort. Each item says why it matters here.

### Security

- [x] **Security headers.** Done in the API (`security_headers` in `main.rs`): nosniff, Referrer-Policy, Permissions-Policy, and for HTML an enforced CSP, `X-Frame-Options: DENY`. Still to do: HSTS as a Cloudflare zone setting. Original note: add `Content-Security-Policy` (allow `challenges.cloudflare.com`
  for Turnstile; start in report-only), `Strict-Transport-Security` (Cloudflare zone setting `security_header`),
  `X-Content-Type-Options: nosniff`, `Referrer-Policy`, `Permissions-Policy`, and `frame-ancestors 'none'`.
  Why: the site renders user-supplied text (shared playgrounds) and loads third-party script.
- [ ] **Validate `Origin` on `/mcp`** (left open on purpose: the server is stateless with no cookies or ambient credentials, so DNS rebinding gains an attacker nothing, and a strict check would block browser-based agents; revisit if auth is ever added) (the MCP spec asks for it, against DNS-rebinding) and narrow CORS. Today
  `CorsLayer::permissive()` applies to everything; keep it for the REST API, restrict it elsewhere.
- [x] **Share creation abuse.** Done: per-client daily quota, total cap with a `capacity` error, token-gated takedown (`DELETE /internal/shares/<id>`), usage in the rollup job and a log warning at 80 %. The warning is emailed through a log-match alert policy in the observability module (needs `notification_email`; apply Terraform). Original note: Shares are public content up to 4 MB each. Add a per-client daily quota, a global
  cap with an alert on database size, and a takedown route (token-gated, like `/internal/jobs/*`). Why: the M10
  has 10 GB and a public link host attracts spam.
- [ ] **Rate limits are per Cloud Run instance.** The token bucket is in memory, so N instances allow N times the
  rate. Rely on the Cloudflare rate-limit rules as the real limit (tune per endpoint: `/api/v1/run` strict,
  discovery documents generous), or move the bucket to Memorystore. The search-run semaphore has the same
  per-instance behaviour; size `max_instances` times the per-instance cap against what the M10 can build.
- [~] **Cloudflare WAF and bots.** Decided: HSTS is set in Terraform (1 month, no preload, no subdomains; raise it later). The Free Managed Ruleset is on by default on the Free plan; the larger Cloudflare Managed and OWASP rulesets need Pro. Bot Fight Mode stays OFF: on the Free plan it cannot be skipped by rules, so it would challenge API and agent clients (this site is built for them). Super Bot Fight Mode (Pro) can be skipped for `/api/`, `/mcp` and `.md`. Original note: Enable the free managed ruleset and Bot Fight Mode, with a skip rule for
  `/api/v1/*` and `/mcp` (query bodies look like attacks; the same reason Cloud Armor excludes them).
- [ ] **Validator hardening.** Top-level aggregation stages are now an allowlist (`ALLOWED_STAGES`, denied by default). Stages nested in `$lookup`, `$unionWith` and `$facet` are checked against the same allowlist. A corpus test (every documented stage is allowed or denied) and a deterministic fuzz of hidden forbidden operators now run in the unit tests. The Cloud Build run uses `FUZZ_ITERATIONS=20000`. Cross-database protection rests on the parser, because Atlas roles cannot match
  `pg_*` names. Confirm unknown aggregation stages and operators are denied by default (an allowlist, not a
  denylist), add fuzzing and a corpus test of every operator in the MongoDB docs, and run it in CI. Consider a
  separate Atlas project for shares and stats so a validator bug cannot reach them.
- [ ] **Supply chain.** Done: Dependabot (`.github/dependabot.yml`); `npm audit` and `cargo audit` run in
  `cloudbuild.yaml` (report only). Still to do: make them blocking or add `cargo deny`; pin the base images by digest;
  sign images and enforce with Binary Authorization.
- [ ] **GitHub.** Done: `CODEOWNERS`. Still to do: branch protection on `main` (pull request and passing checks),
  secret scanning and push protection (still to enable), signed commits.
- [ ] **Credentials.** Rotate every key and token that was pasted into chats; use short-lived Atlas API keys and
  keep the Project Owner key disabled except during an apply; turn on `termination_protection` for the cluster;
  version and restrict the Terraform state bucket.
- [ ] **Origin lock** (section 1).

### Performance

- [x] **Cold starts.** Done in Terraform: `min_instances` (default 1) and `startup_cpu_boost`; set `min_instances = 0` to scale to zero again. Needs `scripts/tf.sh apply`. Original note: `min_instance_count = 0`, so the first request after idle pays a container start and a
  MongoDB connection. Set `min_instance_count = 1` and `startup_cpu_boost`, and connect and ping in a startup
  probe. Costs a few dollars a month; improves the first-visit p95 the most.
- [x] **Prebuilt example datasets.** Done: `api seed-examples` builds `sample_playground` (an Atlas Search and a Vector Search index) and two examples use it (see README). To go live: add `sample_playground` to the sample user's roles (Terraform default does; apply), run the seed once against the cluster with an admin URI, and turn on `FEATURE_SAMPLE_DATA`. Original note: Atlas Search and Vector Search runs take seconds to a minute because the index
  builds first. Keep a few read-only, already-indexed datasets for the built-in examples, so those run instantly;
  only user-supplied data waits for a build.
- [~] **Smaller first load.** Partly done: CodeMirror is its own cached chunk (the app code is now 117 kB, 42 kB gzipped; CodeMirror 504 kB, 169 kB gzipped). The editor is needed on the home page, so lazy-loading it would not speed the first view. Still to do: lazy-load CodeMirror and the docs pages, preload
  the editor chunk, and prerender the home page's text (also the SEO item above).
- [~] **Huge results.** Partly done: the Results panel shows the first 200 KB and says so; Copy and Export keep the full text. Still to do: stream large responses. Original note: Results are not capped (64 MB per statement), and a CodeMirror view of megabytes is slow.
  Render the first N KB in the page with a "download the full result" action; stream large responses.
- [x] **Release build profile.** Done (`[profile.release]` in `Cargo.toml`: thin LTO, one codegen unit, strip). Original note: `Cargo.toml` has no `[profile.release]`: add `lto = "thin"`, `codegen-units = 1`,
  `strip = true` for a smaller, faster binary and a quicker cold start. The container already runs as a non-root user.
- [-] **Per-run database cost.** Decided not to load test for now. Original note: Each run creates and drops a database, which gets expensive on WiredTiger under
  heavy load. If the load test shows stalls, switch to a few reusable "slot" databases with per-run collections.
  Re-run `scripts/loadtest-with-atlas-local.sh` against an M10 first to find the real limit.
- [-] **Tune concurrency.** Decided not to load test for now. Original note: Cloud Run concurrency equals the connection pool size (5). Load-test 8 to 16 with a
  pool of 10 and pick the knee.
- [ ] **Observability.** SLOs and alerts on p95 latency, 5xx rate, Atlas connections, disk and replication lag;
  OpenTelemetry traces to Cloud Trace. The uptime check on `/readyz` exists.
- [-] **Cost.** Decided: keep the load balancer (it carries Cloud Armor and the origin lock). The load balancer and the M10 are the fixed costs. Dropping the load balancer (Cloud Run behind
  Cloudflare directly) saves money but loses Cloud Armor; decide after the origin lock is enforced.

