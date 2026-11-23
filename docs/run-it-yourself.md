# Run it yourself, or get a cluster

Use the hosted playground, run it on your machine against a local MongoDB, or point it at a real Atlas cluster: your own free one, or a temporary ephemeral one. Agents can use it over MCP or the REST API.

> Generated from `crates/core/src/guide.json` by `scripts/gen-run-it-yourself.py` (also served at `GET /api/v1/guide`, by the MCP tool `get_setup_guide`, and as Markdown at `/docs/run-it-yourself.md`). Replace `{origin}` with the site you are using, for example `https://mongoplayground.com` or `http://localhost:8080`.

**Free-forever Atlas cluster (M0, 512 MB, no credit card):** [https://www.mongodb.com/cloud/atlas/register?utm_campaign=devrel&utm_source=third-party-content&utm_medium=cta&utm_content=mongodb-playground&utm_term=diana.esteves](https://www.mongodb.com/cloud/atlas/register?utm_campaign=devrel&utm_source=third-party-content&utm_medium=cta&utm_content=mongodb-playground&utm_term=diana.esteves)

## Which one should I use?

- Just try it, nothing to install: **Use the hosted playground**
- Call it from an AI agent, a script or CI: **Use it from an agent (MCP)**
- Work offline with Atlas Search and Vector Search, no account: **Run locally with the Atlas CLI (no account, includes Atlas Search)**
- Run everything in containers: **Docker Compose**
- A desktop app on a local MongoDB: **Desktop app (Tauri)**
- Keep your data in the cloud, free: **Free-forever Atlas cluster (M0)**
- A temporary cloud cluster for a demo or workshop: **Ephemeral Atlas cluster (temporary, expires in 7 days)**

## Use it now

### Use the hosted playground

Nothing to install. The web app, the REST API and the MCP server are the same service.

1. Open the web app: <{origin}>
2. Run a script through the API

   ```bash
   curl -s -X POST {origin}/api/v1/run -H 'content-type: application/json' -d '{"script": "db.collection.find({age: {$gt: 40}})", "config": "db = { collection: { documents: [{name: \"Ada\", age: 36}, {name: \"Linus\", age: 55}] } }"}'
   ```
3. Machine-readable docs: <{origin}/llms.txt>

### Use it from an agent (MCP)

A stateless Streamable HTTP MCP server. Tools: run_query, validate_query, explain_query, share_query, get_shared, export_playground, list_examples, get_setup_guide, get_limits.

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

## Run it on your machine

### Run locally with the Atlas CLI (no account, includes Atlas Search)

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

### Docker Compose

The API, the web app and an Atlas Local MongoDB (search works) in one command.

1. Start it

   ```bash
   docker compose up --build
   ```
2. Open it: <http://localhost:8080>

### Desktop app (Tauri)

Runs queries in-process against a local MongoDB. No network service.

1. Run it

   ```bash
   cd web && npm install && npm run tauri dev
   ```

## Use a real Atlas cluster

### Free-forever Atlas cluster (M0)

A free cluster in your own Atlas account: 512 MB, no credit card, one per project. It stays until you delete it, so your data persists.

1. Register for Atlas: <https://www.mongodb.com/cloud/atlas/register?utm_campaign=devrel&utm_source=third-party-content&utm_medium=cta&utm_content=mongodb-playground&utm_term=diana.esteves>
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
7. When you are done, delete the cluster if you do not want to keep it

   ```bash
   atlas clusters delete <cluster-name> --force
   ```

### Ephemeral Atlas cluster (temporary, expires in 7 days)

A temporary free Atlas cluster created from the terminal. Atlas pauses it 2 days after creation and deletes it 7 days after creation unless you claim it into an Atlas account, which turns it into a normal free cluster with no expiry. Until it is claimed you can read and write data, but you cannot scale it, add database users or change network access. It runs on AWS us-east-1 and accepts connections from any IP address, so keep sensitive data out. This is a preview Atlas API: it can change, and creating one means you accept the MongoDB Cloud Terms of Service.

1. Check that your Atlas CLI has the command (upgrade the CLI if it is missing)

   ```bash
   atlas api ephemeralClusters createEphemeralCluster --help
   ```
2. Create the cluster. The request body is described in the Atlas Admin API docs; the response has the connection and claim details

   ```bash
   atlas api ephemeralClusters createEphemeralCluster --file request.json
   ```
3. Request body reference (preview API): <https://www.mongodb.com/docs/api/doc/atlas-admin-api-v2/preview/operation/operation-createephemeralcluster?utm_campaign=devrel&utm_source=third-party-content&utm_medium=cta&utm_content=mongodb-playground&utm_term=diana.esteves>
4. Point the API at it, using the connection string from the response

   ```bash
   MONGODB_URI='<connection string from the response>' cargo run -p playground-api
   ```
5. Look it up again later

   ```bash
   atlas api ephemeralClusters getEphemeralCluster --help
   ```
6. Keep it or let it go: claim it within 7 days using the claim details, or do nothing and Atlas deletes it. There is nothing to clean up

_Atlas CLI flags change between releases: check `atlas <command> --help` before relying on these._
