# Run it yourself, or get a cluster

The playground runs on a hosted site, on your machine against a local MongoDB, against your own Atlas cluster (free or ephemeral), as a desktop app, or from an agent over MCP.

> Generated from `crates/core/src/guide.json` (also served at `GET /api/v1/guide` and by the MCP tool `get_setup_guide`). Replace `{origin}` with the site you are using, for example `https://mongoplayground.com` or `http://localhost:8080`.

**Free-forever Atlas cluster (M0, 512 MB, no credit card):** [https://www.mongodb.com/cloud/atlas/register?utm_source=mongoplayground&utm_medium=referral&utm_campaign=playground-free-cluster&utm_content=docs](https://www.mongodb.com/cloud/atlas/register?utm_source=mongoplayground&utm_medium=referral&utm_campaign=playground-free-cluster&utm_content=docs)

## Use the hosted playground

Nothing to install. The web app, the REST API and the MCP server are the same service.

1. Open the web app: <{origin}>
2. Run a script through the API

   ```bash
   curl -s -X POST {origin}/api/v1/run -H 'content-type: application/json' -d '{"script": "db.collection.find({age: {$gt: 40}})", "config": "db = { collection: { documents: [{name: \"Ada\", age: 36}, {name: \"Linus\", age: 55}] } }"}'
   ```
3. Machine-readable docs: <{origin}/llms.txt>

## Use it from an agent (MCP)

A stateless Streamable HTTP MCP server. Tools: run_query, validate_query, explain_query, share_query, get_shared, list_examples, get_limits, get_setup_guide.

1. Claude Code

   ```bash
   claude mcp add --transport http mongodb-playground {origin}/mcp
   ```
2. Any MCP client (JSON config)

   ```bash
   {"mcpServers": {"mongodb-playground": {"type": "http", "url": "{origin}/mcp"}}}
   ```
3. Discovery card: <{origin}/.well-known/mcp.json>
4. OpenAPI 3.1: <{origin}/api/v1/openapi.json>

## Run locally with the Atlas CLI (no account, includes Atlas Search)

A local MongoDB in Docker with Atlas Search and Vector Search, then the API and web app on your machine.

1. Install the Atlas CLI

   ```bash
   brew install mongodb-atlas-cli
   ```
2. Create a local deployment

   ```bash
   atlas local setup playground --port 27017 --force
   ```
3. Run the API (it also serves the web app)

   ```bash
   MONGODB_URI='mongodb://localhost:27017/?directConnection=true' cargo run -p playground-api
   ```
4. Open it: <http://localhost:8080>
5. Delete the deployment when you are done

   ```bash
   atlas local delete playground --force
   ```

## Free-forever Atlas cluster (M0)

512 MB, no credit card, one per project. Good for real, persistent use.

1. Register for Atlas: <https://www.mongodb.com/cloud/atlas/register?utm_source=mongoplayground&utm_medium=referral&utm_campaign=playground-free-cluster&utm_content=docs>
2. Or from the terminal: sign in and create a free cluster

   ```bash
   atlas setup
   ```
3. Allow your IP address

   ```bash
   atlas accessLists create --currentIp
   ```
4. Get the connection string

   ```bash
   atlas clusters connectionStrings describe <cluster-name>
   ```
5. Optional: load the sample datasets (then use Data: Atlas sample data)

   ```bash
   atlas clusters sampleData load <cluster-name>
   ```
6. Point the API at it

   ```bash
   MONGODB_URI='<connection string>' cargo run -p playground-api
   ```

## Ephemeral Atlas cluster (create, use, delete)

A throwaway cloud cluster for a demo or a workshop. Create it, use it, delete it so nothing keeps running.

1. Create a project and a free cluster

   ```bash
   atlas projects create playground-tmp && atlas clusters create playground --tier M0 --provider GCP --region CENTRAL_US
   ```
2. Create a user and allow your IP

   ```bash
   atlas dbusers create readWriteAnyDatabase --username pg --password '<password>' && atlas accessLists create --currentIp
   ```
3. Use it (connection string from the Atlas CLI)

   ```bash
   atlas clusters connectionStrings describe playground
   ```
4. Delete everything

   ```bash
   atlas clusters delete playground --force && atlas projects delete <project-id> --force
   ```

## Docker Compose

The API, the web app and an Atlas Local MongoDB (search works) in one command.

1. Start it

   ```bash
   docker compose up --build
   ```
2. Open it: <http://localhost:8080>

## Desktop app (Tauri)

Runs queries in-process against a local MongoDB. No network service.

1. Run it

   ```bash
   cd web && npm install && npm run tauri dev
   ```

_Atlas CLI flags change between releases: check `atlas <command> --help` before relying on these._
