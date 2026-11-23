---
title: Online MongoDB Playground – Test Queries, Aggregations & Atlas Search
description: Free online MongoDB playground on a real cluster: run mongosh-style queries and aggregations, see query plans, try Atlas Search and Vector Search, share links. No sign-up.
---

# Online MongoDB Playground

**MongoDB Playground is a free online MongoDB playground: paste your own documents, run mongosh-style statements
(find, aggregate, update, delete, indexes, Atlas Search, Vector Search), read the query plan, and share the result
with a link. It runs on a real MongoDB Atlas cluster, needs no account and installs nothing.** Everything the web
app does is also available to scripts and AI agents through a REST API and an MCP server.

Open it: {{origin}}/

## What you can do

- Run **several mongosh-style statements** in order against documents you supply. Each run uses a throwaway database that is deleted afterwards.
- Put **documents, indexes, Atlas Search indexes and Vector Search indexes** in one data panel, per collection.
- Read **query plans** with plain-language advice (index used, documents examined, collection scans).
- **Share** a playground as a UUID link, or **export** it as a mongosh script, JSON, CSV or curl.
- Call everything from code: [REST API]({{origin}}/docs/api.md) and [MCP server]({{origin}}/docs/mcp.md).

## Example

Data panel:

```js
db = { collection: { documents: [
  { name: "Ada", age: 36 }, { name: "Linus", age: 55 }, { name: "Grace", age: 85 }
] } }
```

Shell panel:

```js
db.collection.find({ age: { $gt: 40 } })
```

Result: `Linus` and `Grace`.

## Limits

{{maxCollections}} collections and {{maxDocs}} documents each, {{maxPayloadMb}} MB per request,
{{maxTimeSeconds}} s per statement, {{maxIndexes}} indexes plus 1 Atlas Search and 1 Vector Search index per
collection, no cap on the number of results. `$where`, `$function`, `$out`, `$merge` and references to other
databases are rejected. Share links last {{shareTtlDays}} days after their last view.

## Frequently asked questions

**Is there a free online MongoDB playground?**
Yes. This one is free, needs no sign-up and runs on a real MongoDB Atlas cluster.

**How do I test a MongoDB query online without installing MongoDB?**
Open the playground, paste your documents in the Data panel, write a statement such as
`db.collection.find({ age: { $gt: 40 } })` in the Shell panel, and press Run all.

**How do I run an aggregation pipeline online?**
Write `db.collection.aggregate([ ... ])` in the Shell panel. Add `.explain()` or press Explain plan to see how
MongoDB runs it.

**Can I try Atlas Search or Vector Search without creating a cluster?**
Yes. Declare a `searchIndexes` or `vectorSearchIndexes` entry for a collection in the Data panel and use `$search`
or `$vectorSearch`. The first run is slower because the index has to build first.

**Is it a real MongoDB or a simulator?**
It is a real MongoDB Atlas cluster. Each run uses a throwaway database that is dropped when the run ends.

**Can an AI agent run MongoDB queries here?**
Yes. Use the MCP server at `{{origin}}/mcp` (tools: run_query, validate_query, explain_query, share_query,
get_shared, export_playground, list_examples, get_setup_guide, get_limits) or the REST API at `{{origin}}/api/v1`.

**How do I share a MongoDB query?**
Press Share. You get a link (a random UUID) to your data and statements, valid for {{shareTtlDays}} days after its last view.

## For agents

- Markdown version of every page: add `.md` to the path, or send `Accept: text/markdown`.
- [llms.txt]({{origin}}/llms.txt), [OpenAPI 3.1]({{origin}}/api/v1/openapi.json), [MCP card]({{origin}}/.well-known/mcp.json).
- Pages: [Quickstart]({{origin}}/docs/quickstart.md), [REST API]({{origin}}/docs/api.md), [MCP server]({{origin}}/docs/mcp.md), [Run it yourself]({{origin}}/docs/run-it-yourself.md), [About]({{origin}}/about.md).
- Source code: https://github.com/desteves/mongodb-playground
