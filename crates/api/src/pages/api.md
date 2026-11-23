---
title: MongoDB Playground REST API – Run mongosh Scripts Over HTTP
description: Run mongosh-style scripts, explain queries, share and export playgrounds through a REST API with an OpenAPI 3.1 spec. Built for scripts, CI and AI agents.
---

# REST API

Base URL: `{{origin}}/api/v1`. OpenAPI 3.1: {{origin}}/api/v1/openapi.json. Quick start for agents: {{origin}}/llms.txt.
No account or key is needed.

## Endpoints

| Method | Path | What it does |
|---|---|---|
| POST | `/run` | Run a script against documents you supply and return a transcript |
| POST | `/explain` | Return query plans instead of running the statements |
| POST | `/validate` | Check a request without running it (no database) |
| POST | `/share` | Save a playground; returns a UUID link |
| GET | `/p/{id}` | Load a shared playground |
| POST | `/export` | Export as a mongosh script or JSON |
| GET | `/examples` | Ready-to-run templates and examples (each is a valid `/run` body) |
| GET | `/limits` | Hard limits and capabilities |
| GET | `/guide` | How to run it yourself, with commands |

## Run a script

```bash
curl -s -X POST {{origin}}/api/v1/run -H 'content-type: application/json' -d '{
  "script": "db.collection.find({ age: { $gt: 40 } })",
  "config": "[{ name: \"Ada\", age: 36 }, { name: \"Linus\", age: 55 }]"
}'
```

`script` holds one or more mongosh-style statements, run in order. `config` is the Data panel text: an array (one
collection named `collection`), `db = { name: [...] }`, or `db = { name: { documents, indexes, searchIndexes, vectorSearchIndexes } }`.
A failing statement stops the script and the response keeps the earlier output (`ok: false`).

## Errors and limits

Errors are `{code, message, hint}` with stable codes; follow the `hint`. Work endpoints are rate limited per client:
`429` with `Retry-After` and `X-RateLimit-*` headers. Limits: {{maxCollections}} collections, {{maxDocs}} documents each,
{{maxPayloadMb}} MB per request, {{maxTimeSeconds}} s per statement, {{maxIndexes}} indexes plus 1 Atlas Search and 1 Vector Search index per collection.
Requests and responses may be gzip, br, zstd or deflate compressed.

## Agents

Use the [MCP server]({{origin}}/docs/mcp.md) for tool calling, or this API directly.
