# v0.1.0: first release

An online MongoDB playground for builders, human and agent alike: <https://mongoplayground.com>

## Highlights

- **Shell:** several mongosh-style statements per run (CRUD, aggregation, indexes), a transcript of the output, and Run all / Run selection.
- **Data panel:** your documents, indexes, Atlas Search and Vector Search indexes, per collection.
- **Explain plans** with plain-language advice, on any read or write.
- **Share links:** random UUID links that expire 90 days after the last view. Export a mongosh script, JSON, CSV or curl.
- **Agents first:** versioned REST API (`/api/v1`), OpenAPI 3.1, `llms.txt`, and an MCP server with nine tools.
- **Desktop app:** a Tauri 2 build that runs the same Rust core against a local MongoDB.
- **Infrastructure as code:** Terraform for GCP (Cloud Run, load balancer, Cloud Armor), Atlas (M10 over Private Service Connect) and Cloudflare; Cloud Build for CI/CD.

## Run it

- Locally: see the README ("Run it locally") and `docs/run-it-yourself.md`.
- As an agent: `POST /mcp` or read `/llms.txt`.

## Known limitations

- Terraform is being applied for the first time; expect small fixes.
- The Tauri app compiles but has not been launched, and Turnstile has not been tested against real Cloudflare.
- Sample data is behind a feature flag and off by default.
