//! Desktop shell: the same `playground-core` as the hosted API, run in-process against a local
//! MongoDB (default `mongodb://localhost:27017`; `atlas local setup` gives you one with Atlas
//! Search). No network service, no share links or stats: those need the hosted platform.

use std::{path::PathBuf, sync::Arc};

use playground_core::{Limits, PlaygroundError, RunBody, Runner};
use serde_json::{json, Value};
use tauri::{Manager, State};
use tokio::sync::RwLock;

const DEFAULT_URI: &str = "mongodb://localhost:27017/?directConnection=true";

struct Desktop {
    runner: RwLock<Arc<Runner>>,
    uri: RwLock<String>,
    config_file: PathBuf,
}

/// Errors cross the IPC boundary as the same `{code, message, hint}` object the HTTP API returns.
fn err(e: PlaygroundError) -> Value {
    json!({ "code": e.code(), "message": e.to_string(), "hint": e.hint() })
}

fn bad(msg: impl ToString) -> Value {
    json!({ "code": "invalid_request", "message": msg.to_string(), "hint": "Check the request shape against the examples." })
}

fn request(v: Value) -> Result<RunBody, Value> {
    serde_json::from_value(v).map_err(bad)
}

#[tauri::command]
async fn run_query(state: State<'_, Desktop>, request_body: Value) -> Result<Value, Value> {
    let runner = state.runner.read().await.clone();
    let out = runner
        .run_body(&request(request_body)?)
        .await
        .map_err(err)?;
    serde_json::to_value(out).map_err(bad)
}

#[tauri::command]
async fn explain_query(state: State<'_, Desktop>, request_body: Value) -> Result<Value, Value> {
    let runner = state.runner.read().await.clone();
    let out = runner
        .explain_body(&request(request_body)?)
        .await
        .map_err(err)?;
    serde_json::to_value(out).map_err(bad)
}

#[tauri::command]
async fn validate_query(state: State<'_, Desktop>, request_body: Value) -> Result<Value, Value> {
    let runner = state.runner.read().await.clone();
    runner.validate_body(&request(request_body)?).map_err(err)
}

#[tauri::command]
fn list_examples() -> Value {
    playground_core::examples()
}

#[tauri::command]
async fn get_limits(state: State<'_, Desktop>) -> Result<Value, Value> {
    let runner = state.runner.read().await.clone();
    let l = runner.limits();
    Ok(json!({
        "apiVersion": "desktop",
        "maxCollections": l.max_collections,
        "maxDocsPerCollection": l.max_docs_per_collection,
        "maxPayloadBytes": l.max_payload_bytes,
        "maxTimeMs": l.max_time_ms,
        "maxResults": null,
        "maxIndexesPerCollection": l.max_indexes_per_collection,
        "searchSupported": runner.search_supported().await,
        "sampleSupported": false,
    }))
}

#[tauri::command]
async fn get_connection(state: State<'_, Desktop>) -> Result<String, Value> {
    Ok(state.uri.read().await.clone())
}

/// Point the app at another MongoDB (kept in the app's config folder; a keychain is a later step).
#[tauri::command]
async fn set_connection(state: State<'_, Desktop>, uri: String) -> Result<(), Value> {
    let runner = Runner::connect(&uri, Limits::default())
        .await
        .map_err(err)?;
    *state.runner.write().await = Arc::new(runner);
    *state.uri.write().await = uri.clone();
    if let Some(dir) = state.config_file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(&state.config_file, uri).map_err(bad)?;
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let config_file = app
                .path()
                .app_config_dir()
                .map(|d| d.join("connection.txt"))
                .unwrap_or_else(|_| PathBuf::from("connection.txt"));
            let uri = std::fs::read_to_string(&config_file)
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_URI.to_string());
            // The driver connects lazily, so this works even if MongoDB is not running yet.
            let runner = tauri::async_runtime::block_on(Runner::connect(&uri, Limits::default()))
                .map_err(|e| e.to_string())?;
            app.manage(Desktop {
                runner: RwLock::new(Arc::new(runner)),
                uri: RwLock::new(uri),
                config_file,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            run_query,
            explain_query,
            validate_query,
            list_examples,
            get_limits,
            get_connection,
            set_connection
        ])
        .run(tauri::generate_context!())
        .expect("error while running the desktop app");
}
