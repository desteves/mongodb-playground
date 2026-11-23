# Contributing

Contributions from people and from agents are welcome. Read [AGENTS.md](AGENTS.md) first: it has the code map,
the commands and the rules (agent parity, no secrets, tests on a local Atlas deployment).

## Setup

You need Rust (rustup), Node 22+, and either Docker or the [Atlas CLI](https://www.mongodb.com/docs/atlas/cli/?utm_campaign=devrel&utm_source=third-party-content&utm_medium=cta&utm_content=mongodb-playground&utm_term=diana.esteves)
for a local MongoDB. Details are in the README ("Run it locally").

## Enable the secret guard

```bash
scripts/install-hooks.sh
```

It turns on a pre-commit hook that blocks Terraform plan and state files, `.env` and `*.tfvars` files, and credentials (a MongoDB connection string with a password, private keys, API tokens). Keep Terraform plans outside the repo with `scripts/tf.sh plan` and `scripts/tf.sh apply`.

## Before you open a pull request

```bash
cargo fmt --all && cargo clippy --workspace --exclude playground-desktop --all-targets -- -D warnings
cd web && npm run check && npm test && npm run build
scripts/test-with-atlas-local.sh
scripts/e2e.sh
```

- Add or update tests with the change. Browser behavior gets a Playwright test in `web/e2e`.
- If you change an API, update `crates/api/src/openapi.json`, `crates/api/src/llms.txt` and the MCP tools together.
- Keep pull requests focused, and say what you verified and what you did not.
- Do not include credentials, connection strings or Terraform state.

## Reporting problems

Bugs and ideas: open an issue. Security problems: see [SECURITY.md](SECURITY.md); do not open a public issue.

## Dependency updates

Dependabot opens grouped pull requests weekly (`.github/dependabot.yml`). Every merge to `main` deploys to production and
the pull requests themselves have no checks, so try one on your machine first:

```bash
scripts/try-pr.sh <number>          # Rust and web checks for what the PR touches
scripts/try-pr.sh <number> --full   # plus the Atlas-backed and browser tests
```

Merge it with `gh pr merge <number> --squash --delete-branch` once it passes.
