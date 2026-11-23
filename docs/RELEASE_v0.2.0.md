# v0.2.0: faster first load, safer by default, and agents get every page

<https://mongoplayground.com>

## Highlights

- **Every page for agents:** `Accept: text/markdown` (or `<page>.md`) returns the page as Markdown in Cloudflare's
  Markdown-for-Agents layout. The HTML carries each page's own title, description, canonical URL, structured data
  (FAQPage, HowTo) and text, so crawlers that do not run JavaScript see real content.
- **Prebuilt, already-indexed example datasets:** `api seed-examples` builds `sample_playground` (an Atlas Search and a
  Vector Search index), so the two "prebuilt" examples answer instantly instead of waiting for an index build.
- **Caching:** the origin sends explicit `Cache-Control`, and Cloudflare cache rules cover assets, Markdown twins,
  discovery documents and shares; HTML and the API are never cached.
- **UI:** each panel has its own shade, readable icons, Run only in the Shell, an export preview with copy, a warning
  that the first Search run is slower, a running marker, "Load an example...", and a Results panel that shows the first
  200 KB of very large results (Copy and Export keep all of it).
- **Run it yourself, organised:** options grouped as use it now, on your machine, or on a real Atlas cluster, with the
  ephemeral cluster described correctly (7-day expiry).
- **UTM parameters** on every link back to mongodb.com, from one constant.

## Security

- Security headers on every response, and an enforced Content-Security-Policy on HTML (self and Turnstile only).
- Aggregation stages are now an allowlist (denied by default), including stages nested in `$lookup`, `$unionWith`
  and `$facet`; a corpus test and a fuzz test guard the validator.
- Share links: 30 new links per client per UTC day, a total cap (`503` with code `capacity`), and a token-gated takedown.
  A log-match alert fires at 80 % of the cap.
- Database passwords are ephemeral and write-only in Terraform (never in state or a plan); a pre-commit hook blocks
  plan and state files and credentials.
- Dependabot, CODEOWNERS, and `npm audit` and `cargo audit` in the build log.

## Performance

- Cloud Run keeps one instance warm with startup CPU boost (`min_instances`, default 1).
- CodeMirror is its own cached chunk; the app code is 117 kB (42 kB gzipped).
- Release builds use thin LTO and strip symbols.

## Infrastructure

- Atlas provider 2.x with port-mapped Private Service Connect; Cloudflare cache rules; a Cloud Armor origin lock
  (still in preview, see `docs/todo.md`); `feature_sample_data` and `min_instances` variables.

## Upgrading

- Terraform >= 1.11. Run `scripts/tf.sh plan` and read it before `apply`.
- To use the prebuilt examples: seed the cluster once (`MONGODB_ADMIN_URI=... api seed-examples`), apply Terraform so the
  sample user can read `sample_playground`, and turn on `FEATURE_SAMPLE_DATA` (the Terraform variable does it).
- New environment variables: `SHARES_MAX_TOTAL`, `SHARES_PER_CLIENT_PER_DAY`, `MONGODB_ADMIN_URI` (seeding only).

## Known limitations

- The Tauri desktop app compiles but has not been launched.
- The Cloud Armor origin lock is not enforced yet (`enforce_origin_restriction = false`).
- The Open Graph image still needs rendering once (`scripts/make-og-image.sh`).
