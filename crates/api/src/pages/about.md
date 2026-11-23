---
title: About MongoDB Playground – Runs on a Dedicated MongoDB Atlas Cluster
description: How the online MongoDB playground works: throwaway databases on a dedicated MongoDB Atlas cluster, limits, privacy, and the stack behind it.
---

# About MongoDB Playground

A free online MongoDB playground: paste documents, run mongosh-style commands, see query plans, try Atlas Search and
Vector Search, then share or export. Built for people and for AI agents.

## It runs on a MongoDB Atlas {{cluster}} cluster

Every run executes on a dedicated MongoDB Atlas {{cluster}} cluster, reached privately from the application (no
public network path). Your documents are loaded into a throwaway database, your statements run against it in order,
and the database is dropped when the run ends. Share links and anonymous usage counts live in separate databases on
the same cluster, each with its own database user.

## What is stored

- Share links: the playground you chose to share, public, for {{shareTtlDays}} days after the last view.
- Usage counts: what kind of action, how long it took, whether it worked. Never your queries or documents, never raw IP addresses.

## Limits

{{maxCollections}} collections and {{maxDocs}} documents each, {{maxPayloadMb}} MB payload, {{maxTimeSeconds}} s per
statement, {{maxIndexes}} indexes plus 1 Atlas Search and 1 Vector Search index per collection, no cap on the number of results.

## What is blocked

Server-side JavaScript (`$where`, `$function`), `$out` and `$merge`, admin stages, and references to other databases.

## How it is built

- A Rust service (axum and the MongoDB Rust driver) with the shell parser, validation and the runner.
- A Svelte and CodeMirror web app, also packaged as a desktop app with Tauri.
- Cloud Run behind a Cloudflare-fronted load balancer, secrets in Secret Manager, everything defined in Terraform.
- A REST API with an OpenAPI spec, and an MCP server, so agents can use everything the web app can.

## Open source

Code, Terraform and docs: https://github.com/desteves/mongodb-playground/tree/main. Issues and pull requests are
welcome, from people and from agents.

## Use your own cluster

Want your data to stick around? Create a free Atlas cluster: https://www.mongodb.com/cloud/atlas/register, then
[run the playground against it]({{origin}}/docs/run-it-yourself.md).

## Terms of use

Don't paste secrets or personal data: shared links are public and runs are logged on the cluster for operations.
This project is independent; MongoDB and Atlas are trademarks of MongoDB, Inc.
