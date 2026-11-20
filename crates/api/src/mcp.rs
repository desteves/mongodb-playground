//! A minimal, stateless MCP server (Streamable HTTP, JSON responses only) so agents can
//! discover and run playground queries without writing an HTTP client.
//!
//! Tools: `run_query`, `validate_query`, `list_examples`, `get_limits`.

use playground_core::{PlaygroundError, RunBody, SharedPlayground};
use serde_json::{json, Value};

use crate::{error_json, limits_json, AppState};

const DEFAULT_PROTOCOL: &str = "2025-06-18";

/// Tool names in the order `tools/list` returns them.
pub const TOOL_NAMES: [&str; 9] = [
    "run_query",
    "validate_query",
    "explain_query",
    "share_query",
    "get_shared",
    "export_playground",
    "list_examples",
    "get_setup_guide",
    "get_limits",
];

const QUERY_SHAPES: &str = "Preferred: `script`, a mongosh-style script run in order against a throwaway database, e.g. \
`db.people.insertMany([{name: 'Ada', age: 36}]); db.people.find({age: {$gt: 30}}).sort({age: -1}); db.people.createIndex({age: 1}); db.people.find({age: 36}).explain()`. \
Supported: find/findOne (+ sort, limit, skip, projection, count, explain), aggregate, countDocuments, estimatedDocumentCount, distinct, insertOne/insertMany, updateOne/updateMany/replaceOne, \
deleteOne/deleteMany, createIndex (max 3 per collection), dropIndex, getIndexes, createSearchIndex (1 search + 1 vector search per collection), at most 25 statements. \
Values use shell syntax (unquoted keys, 'single quotes', /regex/i, ObjectId(), ISODate(), NumberLong()) or plain JSON. \
`config` is the data panel: {collection: {documents: [..], indexes: [{key: {age: 1}}], searchIndexes: [{definition: {mappings: {dynamic: true}}}], vectorSearchIndexes: [{definition: {fields: [{type: 'vector', path: 'embedding', numDimensions: 3, similarity: 'cosine'}]}}]}}, \
or just an array of documents (one collection named `collection`), optionally written `db = {...}`. \
Set `explain` (queryPlanner|executionStats|allPlansExecution) to explain the statements instead of running them (inserts and index builds still run). \
The result is a transcript: `statements[]` each with results/write/explain/error; `ok` is false if a statement failed (earlier output is kept). \
Structured fallback (one operation): mode (find|aggregate|count|distinct|insert|update|delete|listIndexes), collection, collections, query, indexes, searchIndexes. \
$where, $function, $out, $merge and cross-database references are rejected.";

/// Handle one JSON-RPC message. Returns `None` for notifications (no reply).
pub async fn handle(state: &AppState, msg: Value) -> Option<Value> {
    let id = msg.get("id").cloned()?;
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);

    let result: Result<Value, Value> = match method {
        "initialize" => Ok(initialize(&params)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => Ok(call_tool(state, &params).await),
        other => Err(json!({ "code": -32601, "message": format!("method not found: {other}") })),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
    })
}

fn initialize(params: &Value) -> Value {
    let version = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_PROTOCOL);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "mongodb-playground", "version": env!("CARGO_PKG_VERSION") },
        "instructions": "Call get_limits and list_examples first, then validate_query (no database) and run_query. Errors carry a `hint`."
    })
}

fn run_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "script": { "type": "string", "description": "mongosh-style statements, e.g. db.people.find({age: {$gt: 30}}). Several statements run in order." },
            "config": {
                "description": "The data panel: {collection: {documents, indexes, searchIndexes, vectorSearchIndexes}} or an array of documents (collection named `collection`). A JSON value, or a string in shell syntax (`db = {...}`). `collections` is an alias.",
                "oneOf": [{ "type": "object" }, { "type": "array" }, { "type": "string" }]
            },
            "explain": { "type": "string", "enum": ["queryPlanner", "executionStats", "allPlansExecution"], "description": "Explain the statements instead of running them." },
            "datasource": {
                "type": "object",
                "description": "{\"sample\": \"sample_mflix\"} runs read-only against an Atlas sample database (omit config; results are capped).",
                "properties": { "sample": { "type": "string" } }
            },
            "mode": { "type": "string", "enum": ["find", "aggregate", "count", "distinct", "insert", "update", "delete", "listIndexes"], "description": "Structured fallback (instead of `script`): the single operation." },
            "collection": { "type": "string", "description": "Structured fallback: the collection the operation runs on." },
            "query": { "description": "Structured fallback: shape depends on mode." },
            "indexes": { "type": "object", "description": "Structured fallback: collection -> [{key, options?}], at most 3 per collection." },
            "searchIndexes": { "type": "object", "description": "Structured fallback: collection -> {name?, definition: {mappings}}." }
        }
    })
}

fn tools() -> Value {
    let empty = json!({ "type": "object", "properties": {} });
    json!([
        {
            "name": "run_query",
            "description": format!("Run a MongoDB find, aggregate, count, distinct, insert, update or delete against documents you supply, in a throwaway database. Returns results as Extended JSON. {QUERY_SHAPES}"),
            "inputSchema": run_schema()
        },
        {
            "name": "validate_query",
            "description": format!("Check a request against the playground's rules and limits without running it (no database access). Use before run_query. {QUERY_SHAPES}"),
            "inputSchema": run_schema()
        },
        {
            "name": "explain_query",
            "description": format!("Return only the query plan (no results) for a request, with a plain-language summary (index used, COLLSCAN, documents examined) and advice. Writes are explained, not performed. Defaults to executionStats. Insert cannot be explained. {QUERY_SHAPES}"),
            "inputSchema": run_schema()
        },
        {
            "name": "share_query",
            "description": "Save a playground (same arguments as run_query) under a permanent link. Returns {id, url}; the url is /p/<id> on the playground site and GET /api/v1/p/<id> returns it. Links expire after 90 days without a view. Only valid playgrounds can be shared.",
            "inputSchema": run_schema()
        },
        {
            "name": "get_shared",
            "description": "Load a shared playground by id. The result can be passed to run_query as-is (without id).",
            "inputSchema": { "type": "object", "required": ["id"], "properties": { "id": { "type": "string" } } }
        },
        {
            "name": "export_playground",
            "description": "Take a playground out of the site: format \"mongosh\" returns a script that recreates the data, indexes and statements against any MongoDB (run with mongosh --file); \"json\" returns the data panel as Extended JSON. Arguments: script, config (the data panel), format.",
            "inputSchema": { "type": "object", "properties": {
                "script": { "type": "string" },
                "config": { "description": "The data panel (see run_query)." },
                "format": { "type": "string", "enum": ["mongosh", "json"] }
            } }
        },
        {
            "name": "list_examples",
            "description": "List ready-to-run example requests (find, aggregate, $lookup, update). Each can be passed to run_query as-is.",
            "inputSchema": empty
        },
        {
            "name": "get_setup_guide",
            "description": "How to run the playground yourself (locally with the Atlas CLI, Docker, the desktop app), get a free-forever or an ephemeral Atlas cluster (with the sign-up link and exact commands), or connect an agent over MCP.",
            "inputSchema": empty
        },
        {
            "name": "get_limits",
            "description": "Return the playground limits (collections, documents, payload size, result count, query time).",
            "inputSchema": empty
        }
    ])
}

async fn call_tool(state: &AppState, params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    match name {
        "get_limits" => ok(limits_json(state).await),
        "export_playground" => {
            let req: playground_core::ExportRequest = match serde_json::from_value(args) {
                Ok(r) => r,
                Err(e) => {
                    return tool_error(&PlaygroundError::Invalid(format!("bad arguments: {e}")))
                }
            };
            match playground_core::export(&req) {
                Ok(out) => ok(
                    json!({ "format": out.format, "filename": out.filename, "mimeType": out.mime_type, "content": out.content }),
                ),
                Err(e) => tool_error(&e),
            }
        }
        "get_setup_guide" => ok(playground_core::guide_for(&state.public_url)),
        "get_shared" => {
            let id = args.get("id").and_then(Value::as_str).unwrap_or("");
            match state.shares.load(id).await {
                Ok(loaded) => {
                    let mut body = serde_json::to_value(&loaded.playground).unwrap_or(Value::Null);
                    body["id"] = json!(id);
                    ok(body)
                }
                Err(e) => tool_error(&e),
            }
        }
        "share_query" => {
            let playground: SharedPlayground = match serde_json::from_value(args) {
                Ok(p) => p,
                Err(e) => {
                    return tool_error(&PlaygroundError::Invalid(format!("bad arguments: {e}")))
                }
            };
            match state.shares.save(&playground, state.runner.limits()).await {
                Ok(id) => ok(json!({ "id": id, "url": format!("/p/{id}") })),
                Err(e) => tool_error(&e),
            }
        }
        "list_examples" => ok(json!({ "examples": state.visible_examples() })),
        "validate_query" | "run_query" | "explain_query" => {
            let body: RunBody = match serde_json::from_value(args) {
                Ok(b) => b,
                Err(e) => {
                    return tool_error(&PlaygroundError::Invalid(format!("bad arguments: {e}")))
                }
            };
            if let Err(e) = state.check_features(&body) {
                return tool_error(&e);
            }
            let result = match name {
                "validate_query" => state.runner.validate_body(&body),
                "explain_query" => state
                    .runner
                    .explain_body(&body)
                    .await
                    .map(|r| serde_json::to_value(&r).unwrap_or(Value::Null)),
                _ => state
                    .runner
                    .run_body(&body)
                    .await
                    .map(|r| serde_json::to_value(&r).unwrap_or(Value::Null)),
            };
            match result {
                // A script that stopped at a failing statement still returns its transcript.
                Ok(v) if v.get("ok") == Some(&json!(false)) => failed_run(v),
                Ok(v) => ok(v),
                Err(e) => tool_error(&e),
            }
        }
        other => tool_error(&PlaygroundError::Invalid(format!(
            "unknown tool `{other}`; call tools/list to see the available tools"
        ))),
    }
}

fn ok(structured: Value) -> Value {
    json!({
        "content": [{ "type": "text", "text": structured.to_string() }],
        "structuredContent": structured,
        "isError": false
    })
}

fn failed_run(structured: Value) -> Value {
    json!({
        "content": [{ "type": "text", "text": structured.to_string() }],
        "structuredContent": structured,
        "isError": true
    })
}

fn tool_error(e: &PlaygroundError) -> Value {
    let body = error_json(e);
    json!({
        "content": [{ "type": "text", "text": body.to_string() }],
        "structuredContent": body,
        "isError": true
    })
}

#[cfg(test)]
mod tests {
    use playground_core::{Limits, Runner, Shares, Stats};

    use super::*;

    async fn state() -> AppState {
        // The driver connects lazily, so nothing here needs a running MongoDB.
        let runner = Runner::connect(
            "mongodb://localhost:1/?serverSelectionTimeoutMS=200",
            Limits::default(),
        )
        .await
        .unwrap();
        let shares = Shares::new(runner.client().clone(), "playground_shares_test");
        let stats_client = runner.client().clone();
        AppState {
            runner,
            shares,
            stats: Stats::disabled(),
            examples: playground_core::examples(),
            internal_token: None,
            stats_salt: "test".into(),
            stats_db: "test".into(),
            stats_client,
            public_url: "https://playground.test".into(),
            features: crate::Features { sample_data: true },
            guard: crate::guard::Guard {
                limiter: crate::guard::RateLimiter::new(crate::guard::RateConfig::from_env(|_| {
                    None
                })),
                verify_limiter: crate::guard::RateLimiter::new(crate::guard::RateConfig {
                    basic: (5.0, 0.1),
                    verified: (5.0, 0.1),
                }),
                site_key: None,
                session_secret: b"test".to_vec(),
                verifier: None,
            },
        }
    }

    async fn rpc(state: &AppState, method: &str, params: Value) -> Value {
        handle(
            state,
            json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}),
        )
        .await
        .expect("a reply")
    }

    fn valid_args() -> Value {
        json!({
            "mode": "find",
            "collection": "people",
            "collections": {"people": [{"name": "Ada"}]},
            "query": {"filter": {"name": "Ada"}}
        })
    }

    #[tokio::test]
    async fn notifications_get_no_reply() {
        let s = state().await;
        assert!(handle(
            &s,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .await
        .is_none());
    }

    #[tokio::test]
    async fn initialize_echoes_protocol_version() {
        let s = state().await;
        let r = rpc(&s, "initialize", json!({"protocolVersion": "2025-03-26"})).await;
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(r["result"]["serverInfo"]["name"], "mongodb-playground");
    }

    #[tokio::test]
    async fn lists_the_tools_with_schemas() {
        let s = state().await;
        let r = rpc(&s, "tools/list", json!({})).await;
        let names: Vec<&str> = r["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, TOOL_NAMES);
        assert!(r["result"]["tools"][0]["inputSchema"]["properties"]["mode"].is_object());
    }

    #[tokio::test]
    async fn unknown_method_is_a_jsonrpc_error() {
        let s = state().await;
        let r = rpc(&s, "nope", json!({})).await;
        assert_eq!(r["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn get_limits_and_examples_work_without_a_database() {
        let s = state().await;
        let r = rpc(&s, "tools/call", json!({"name": "get_limits"})).await;
        assert_eq!(
            r["result"]["structuredContent"]["maxDocsPerCollection"],
            256
        );
        assert!(r["result"]["structuredContent"]["maxResults"].is_null());
        let r = rpc(&s, "tools/call", json!({"name": "list_examples"})).await;
        assert!(
            r["result"]["structuredContent"]["examples"]
                .as_array()
                .unwrap()
                .len()
                >= 4
        );
    }

    #[tokio::test]
    async fn every_example_validates() {
        let s = state().await;
        for ex in s.examples.as_array().unwrap() {
            let mut args = json!({ "script": ex["script"] });
            for (from, to) in [("config", "config"), ("datasource", "datasource")] {
                if ex.get(from).is_some() {
                    args[to] = ex[from].clone();
                }
            }
            let r = rpc(
                &s,
                "tools/call",
                json!({"name": "validate_query", "arguments": args}),
            )
            .await;
            assert_eq!(
                r["result"]["isError"], false,
                "example {} failed: {r}",
                ex["id"]
            );
        }
    }

    #[tokio::test]
    async fn validate_reports_summary() {
        let s = state().await;
        let r = rpc(
            &s,
            "tools/call",
            json!({"name": "validate_query", "arguments": valid_args()}),
        )
        .await;
        let out = &r["result"]["structuredContent"];
        assert_eq!(out["valid"], true);
        assert_eq!(out["documentCounts"]["people"], 1);
    }

    #[tokio::test]
    async fn forbidden_query_returns_error_with_hint() {
        let s = state().await;
        let mut args = valid_args();
        args["query"] = json!({"filter": {"$where": "1"}});
        let r = rpc(
            &s,
            "tools/call",
            json!({"name": "validate_query", "arguments": args}),
        )
        .await;
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(r["result"]["structuredContent"]["code"], "forbidden");
        assert!(!r["result"]["structuredContent"]["hint"]
            .as_str()
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn bad_arguments_and_unknown_tool_are_tool_errors() {
        let s = state().await;
        let r = rpc(
            &s,
            "tools/call",
            json!({"name": "run_query", "arguments": {"mode": "find"}}),
        )
        .await;
        assert_eq!(r["result"]["isError"], true);
        let r = rpc(&s, "tools/call", json!({"name": "drop_everything"})).await;
        assert_eq!(r["result"]["isError"], true);
    }

    #[tokio::test]
    async fn text_inputs_with_regex_literals_and_indexes_validate() {
        let s = state().await;
        let args = json!({
            "mode": "find",
            "collection": "people",
            "collections": "{\"people\": [{\"name\": \"Ada\"}]} // sample",
            "query": "{\"filter\": {\"name\": /^a/i}}",
            "indexes": {"people": [{"key": {"name": 1}}]}
        });
        let r = rpc(
            &s,
            "tools/call",
            json!({"name": "validate_query", "arguments": args}),
        )
        .await;
        assert_eq!(r["result"]["isError"], false, "{r}");
    }

    #[tokio::test]
    async fn a_fourth_index_is_a_tool_error_with_a_hint() {
        let s = state().await;
        let idx: Vec<_> = ["a", "b", "c", "d"]
            .iter()
            .map(|f| json!({"key": {*f: 1}}))
            .collect();
        let mut args = valid_args();
        args["indexes"] = json!({"people": idx});
        let r = rpc(
            &s,
            "tools/call",
            json!({"name": "validate_query", "arguments": args}),
        )
        .await;
        assert_eq!(r["result"]["isError"], true);
        assert!(r["result"]["structuredContent"]["message"]
            .as_str()
            .unwrap()
            .contains("limit is 3"));
    }

    #[tokio::test]
    async fn setup_guide_tool_fills_in_the_public_url_and_atlas_utms() {
        let s = state().await;
        let r = rpc(&s, "tools/call", json!({"name": "get_setup_guide"})).await;
        assert_eq!(r["result"]["isError"], false);
        let text = r["result"]["structuredContent"].to_string();
        assert!(
            text.contains(
                "claude mcp add --transport http mongodb-playground https://playground.test/mcp"
            ),
            "{text}"
        );
        assert!(
            text.contains("utm_source=mongoplayground") && text.contains("utm_content=guide"),
            "{text}"
        );
        assert!(!text.contains("{origin}") && !text.contains("{atlasRegister}"));
    }
}
