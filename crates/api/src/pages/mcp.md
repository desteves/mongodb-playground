---
title: MongoDB Playground MCP Server – Test MongoDB Queries from AI Agents
description: Connect Claude, Cursor or any MCP client to a MongoDB query playground. Tools to run, validate, explain, share and export mongosh scripts. No sign-up.
---

# MCP server

An MCP server (Streamable HTTP, stateless, JSON responses) so AI agents can run, validate, explain, share and export
mongosh-style MongoDB scripts. No account or key needed.

- Endpoint: `{{origin}}/mcp` (also `{{origin}}/api/v1/mcp`)
- Discovery card: {{origin}}/.well-known/mcp.json

## Connect

```bash
claude mcp add --transport http mongodb-playground {{origin}}/mcp
```

Any MCP client that supports Streamable HTTP can use the endpoint URL directly.

## Tools

| Tool | What it does |
|---|---|
| `run_query` | Run a script (several statements, in order) and return a transcript |
| `validate_query` | Check a request without running it |
| `explain_query` | Return query plans instead of running the statements |
| `share_query` | Save a playground and return its UUID link |
| `get_shared` | Load a shared playground |
| `export_playground` | Export as a mongosh script or JSON |
| `list_examples` | Ready-to-run examples (each can be passed to `run_query`) |
| `get_setup_guide` | How to run it yourself, with commands |
| `get_limits` | Hard limits and capabilities |

Suggested flow: `get_limits`, `list_examples`, `validate_query`, then `run_query` or `explain_query`, then `share_query`.
Errors come back as `{code, message, hint}`; follow the hint.

The same operations are available over the [REST API]({{origin}}/docs/api.md).
