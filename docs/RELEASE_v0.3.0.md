# v0.3.0: variables, Format, clearer results, and a tighter edge

<https://mongoplayground.com>

## Highlights

- **Variables and stages as arguments.** The Shell accepts `var`, `let` and `const` (and plain reassignment), usable inside
  any later value, and `aggregate(stage, stage, ...)` as well as `aggregate([stages], {options})`. There is a new example,
  "Variables: reuse stages across aggregations (find duplicates)". Limits keep variables from being a way to blow up memory:
  50 per script, 1 MB each, 8 MB in total once copied into statements.
- **Format.** A Format button on the Data and Shell panels (Shift+Alt+F inside an editor) tidies indentation and line breaks.
  Short objects, arrays and calls stay on one line, longer ones break one item per line. Comments, strings and regexes are
  kept, only whitespace and trailing commas change, and the result is checked against the input token by token before it is
  used. Undo brings the old text back. Agents get `POST /api/v1/format` (OpenAPI and `llms.txt` updated); the desktop app has
  the same pass.
- **Results say less.** Each block starts with `// results for the command on line N` instead of repeating the query, and the
  line number counts from the top of the Shell even when you run only a highlighted part. Export keeps the full transcript.
- **Highlighted text is visible.** The editors' current-line background used to cover the selection (it is drawn behind the
  text); it is now translucent, and the selection colours are stronger in both themes.
- **The prebuilt dataset is visible.** Opening a prebuilt Search or Vector Search example shows `sample_playground` read-only
  in the Data panel, in the panel's own shape. Agents get the same from `GET /api/v1/samples/sample_playground`.
- **Real 404s.** Made-up URLs (spam links, scanners) now return `404` instead of `200` with the app's HTML, so search engines do
  not treat them as pages of the site.
- **Run it yourself reads better.** The "Which one should I use?" links jump to their option, and steps with a link show the
  step's label instead of a raw URL, on the page and in the Markdown pages.
- **Social card.** Open Graph and Twitter tags with a 1200x630 image.

## Security and infrastructure

- **Cloud Armor origin lock, stage 1.** Requests without our secret `X-Origin-Key` header get `403` at the load balancer, so
  nothing reaches it except through the Cloudflare zone. Stage 2 (deny sources outside Cloudflare's published ranges) stays in
  preview until the logs have been clean for a few days. The two stages are separate Terraform variables
  (`enforce_origin_key`, `enforce_origin_restriction`).
- **Cloudflare:** HSTS (one month), a rule that keeps verified crawlers out of Cloudflare's own challenges, and cache rules for
  the social image and theme script.
- **Alerts** by email for downtime and for share storage above 80 % of its cap; share links are limited per client per day and
  in total, with a token-gated takedown route.
- **Faster deploys.** The deploy step waits only for the image push; the slower `cargo audit` runs beside it and only reports.
- **Stricter validation.** Aggregation stages are an allowlist (top level and nested), with a fuzz test in CI.

## Dependencies

- Vite 8 (Rolldown) and `@sveltejs/vite-plugin-svelte` 7; `svelte` 5.59; `mongodb` 3.10.2; `tower-http` 0.7; `hmac` 0.13 with
  `sha2` 0.11. TypeScript stays on 5.x because `svelte-check` does not support 7 yet.
- Dependabot now groups its pull requests (the crypto crates together, Vite with its Svelte plugin together), and
  `scripts/try-pr.sh <number>` runs the checks on a pull request before you merge it. `scripts/bump-version.sh` sets the version
  everywhere in one step.

## Upgrading

- Terraform >= 1.11. Run `scripts/tf.sh plan` and read it before `apply`. New variables: `min_instances` (default 1),
  `feature_sample_data` (default on), `enforce_origin_key`.
- To use the prebuilt examples on your own cluster: seed it once (`MONGODB_ADMIN_URI=... api seed-examples`), let the sample
  user read `sample_playground`, and keep `FEATURE_SAMPLE_DATA` on. An example is only offered when its database exists.
- New environment variables: `SHARES_MAX_TOTAL`, `SHARES_PER_CLIENT_PER_DAY`, `MONGODB_ADMIN_URI` (seeding only).

## Known limitations

- **Run selection** sends only the highlighted text, so variables declared outside the selection are not available to it.
- The Tauri desktop app compiles but has not been launched.
- Cloud Armor stage 2 is not enforced yet.
- `vite.config.ts` still uses `build.rollupOptions.output.manualChunks`, which Vite 8 treats as deprecated (it works); the
  replacement is `build.rolldownOptions.output.codeSplitting`.
