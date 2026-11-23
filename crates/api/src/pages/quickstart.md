---
title: MongoDB Playground Quickstart – Test a MongoDB Query in 60 Seconds
description: Learn the online MongoDB playground in minutes: run find, aggregate and update commands, read query plans, add indexes, try Atlas Search and Vector Search.
---

# Quickstart

Test a MongoDB query in about a minute: no install, no account. Open {{origin}}/ and follow these steps. Each
example below also opens ready to run at `{{origin}}/?example=<id>`.

## The three panels

- **Data**: input data to test: your documents, plus indexes, Atlas Search and Vector Search indexes.
- **Shell**: run mongosh commands: CRUD operations, aggregations, query plans, Vector Search and more.
- **Results**: the output of each statement, the plans, and any error with a hint.

## 1. Run your first query

The playground starts with a small collection and `db.collection.find({ age: { $gt: 40 } })`. Press **Run all** (or
Ctrl/Cmd + Enter). The result lists the matching documents. Example id: `single`.

## 2. Write several statements

Statements run in order against the same data, so an insert is visible to the next query.

```js
db.collection.insertOne({ name: "Tim", age: 30, city: "London" })
db.collection.updateMany({ city: "London" }, { $set: { country: "UK" } })
db.collection.find({ country: "UK" }, { _id: 0, name: 1 }).sort({ name: 1 })
db.collection.countDocuments({})
```

**Run all** runs everything in the Shell; **Run selection** runs only highlighted statements. If a statement fails,
the script stops there and keeps the output so far. Example id: `crud-session`.

## 3. See how MongoDB runs it

**Explain plan** shows the query plan instead of running the statements: whether an index was used, how many
documents were examined, and plain-language advice. Add an index in the Data panel and explain again. Example id: `index-explain`.

## 4. Add more data and indexes

One entry per collection, each with documents and indexes. Example id: `multi`.

```js
db = {
  orders:    { documents: [ ... ], indexes: [ { key: { item: 1 } } ] },
  inventory: { documents: [ ... ] }
}
```

## 5. Try Atlas Search and Vector Search

Declare a `searchIndexes` or `vectorSearchIndexes` entry for a collection and use `$search` or `$vectorSearch`.
The first run is slower (up to about a minute) because the index has to build before the query runs.
Example ids: `atlas-search`, `vector-search`.

## 6. Share and export

- **Share** creates a UUID link to your data and statements, valid for {{shareTtlDays}} days after its last view.
- **Export** gives a mongosh script, the data as JSON, the results as JSON or CSV, or a curl command.

## 7. Use it from code or an agent

Everything the page does is an API call. See the [REST API]({{origin}}/docs/api.md) and the [MCP server]({{origin}}/docs/mcp.md).

## Keep your data

The playground uses throwaway databases. To keep data, create your own cluster and run the playground against it:
[Run it yourself]({{origin}}/docs/run-it-yourself.md). Free Atlas cluster: https://www.mongodb.com/cloud/atlas/register

## Cheat sheet

| Group | Supported |
|---|---|
| Read | `find`, `findOne`, `aggregate`, `countDocuments`, `distinct` |
| Write | `insertOne/Many`, `updateOne/Many`, `replaceOne`, `deleteOne/Many` |
| Indexes | `createIndex`, `dropIndex`, `getIndexes`, `createSearchIndex` |
| Cursor | `.sort()` `.limit()` `.skip()` `.projection()` `.count()` `.explain()` |
| Values | unquoted keys, `'single quotes'`, `/regex/i`, `ObjectId()`, `ISODate()`, comments |
