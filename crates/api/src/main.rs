mod guard;
mod mcp;

use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::{DefaultBodyLimit, FromRequest, Path, Request, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use playground_core::{
    make_client, stats, Limits, PlaygroundError, Reply, RunBody, Runner, SharedPlayground, Shares,
    Stats,
};
use serde_json::{json, Value};
use tower_http::{
    compression::CompressionLayer,
    cors::CorsLayer,
    decompression::RequestDecompressionLayer,
    services::{ServeDir, ServeFile},
    set_header::SetResponseHeaderLayer,
    trace::TraceLayer,
};
use tracing_subscriber::EnvFilter;

/// Switches for unfinished or optional features. Off unless the environment turns them on.
pub(crate) struct Features {
    /// Query Atlas's sample datasets read-only (`FEATURE_SAMPLE_DATA=1`).
    pub sample_data: bool,
}

impl AppState {
    /// Reject a request that uses a feature this deployment has turned off.
    /// Templates and examples this deployment can actually run (sample-data ones need the feature flag).
    pub(crate) fn visible_examples(&self) -> Value {
        match &self.examples {
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .filter(|e| self.features.sample_data || e.get("datasource").is_none())
                    .cloned()
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    pub(crate) fn check_features(&self, body: &RunBody) -> Result<(), PlaygroundError> {
        let sample = match body {
            RunBody::Script(r) => r.datasource.is_some(),
            RunBody::Structured(r) => r.datasource.is_some(),
        };
        if sample && !self.features.sample_data {
            return Err(PlaygroundError::Invalid(
                "sample data is turned off on this deployment (feature flag FEATURE_SAMPLE_DATA); paste your own documents into the data panel".into(),
            ));
        }
        Ok(())
    }
}

pub(crate) struct AppState {
    pub features: Features,
    /// Rate limits and the Turnstile session tier.
    pub guard: guard::Guard,
    pub runner: Runner,
    pub shares: Shares,
    pub stats: Stats,
    pub examples: Value,
    /// Header value that authorises `/internal/jobs/*`; the endpoints 404 when unset.
    pub internal_token: Option<String>,
    pub stats_salt: String,
    pub stats_db: String,
    pub stats_client: mongodb::Client,
    /// Public URL of this server (`PUBLIC_URL`), used in the setup guide and the MCP card.
    pub public_url: String,
}

pub(crate) type Shared = Arc<AppState>;

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

#[tokio::main]
async fn main() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    // Cloud Run parses structured JSON logs; plain text is nicer locally.
    if env("LOG_JSON").is_some() {
        builder.json().init();
    } else {
        builder.init();
    }

    let uri = env("MONGODB_URI").unwrap_or_else(|| {
        eprintln!("MONGODB_URI is not set; defaulting to mongodb://localhost:27017");
        "mongodb://localhost:27017".to_string()
    });
    let port: u16 = env("PORT").and_then(|p| p.parse().ok()).unwrap_or(8080);
    let host = env("BIND").unwrap_or_else(|| "0.0.0.0".to_string());

    let limits = Limits::default();
    // Body limit applies to the *decompressed* body (the decompression layer sits outside it),
    // with slack for the JSON envelope around the documents.
    let body_limit = limits.max_payload_bytes + 1024 * 1024;
    // Separate users in production: runner (temp databases), sample (read-only), shares, stats.
    let runner = Runner::connect_with(&uri, env("MONGODB_SAMPLE_URI").as_deref(), limits)
        .await
        .expect("invalid MONGODB_URI");

    let shares_client = match env("MONGODB_SHARES_URI") {
        Some(u) => make_client(&u).await.expect("invalid MONGODB_SHARES_URI"),
        None => runner.client().clone(),
    };
    let shares = Shares::new(
        shares_client,
        env("SHARES_DB").unwrap_or_else(|| "playground_shares".into()),
    );

    let stats_client = match env("STATS_MONGODB_URI") {
        Some(u) => make_client(&u).await.expect("invalid STATS_MONGODB_URI"),
        None => runner.client().clone(),
    };
    let stats_db = env("STATS_DB").unwrap_or_else(|| "playground-platform".into());
    let stats = if env("STATS_DISABLED").is_some() {
        Stats::disabled()
    } else {
        Stats::start(
            stats_client.clone(),
            stats_db.clone(),
            env("APP_ENV").unwrap_or_else(|| "dev".into()),
        )
    };

    let state = Arc::new(AppState {
        runner,
        shares,
        stats,
        examples: playground_core::examples(),
        internal_token: env("INTERNAL_TOKEN"),
        stats_salt: env("STATS_SALT").unwrap_or_else(|| "dev-salt".into()),
        stats_db,
        stats_client,
        public_url: env("PUBLIC_URL").unwrap_or_else(|| format!("http://localhost:{port}")),
        features: Features {
            sample_data: env("FEATURE_SAMPLE_DATA")
                .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "on" | "yes")),
        },
        guard: guard::Guard {
            limiter: guard::RateLimiter::new(guard::RateConfig::from_env(env)),
            verify_limiter: guard::RateLimiter::new(guard::RateConfig {
                basic: (5.0, 0.1),
                verified: (5.0, 0.1),
            }),
            site_key: env("TURNSTILE_SITE_KEY"),
            // One secret for all instances (set SESSION_SECRET); a random one only works for a single instance.
            session_secret: env("SESSION_SECRET")
                .unwrap_or_else(uuid_secret)
                .into_bytes(),
            verifier: env("TURNSTILE_SECRET")
                .map(|secret| guard::cloudflare_verifier(secret, env("TURNSTILE_VERIFY_URL"))),
        },
    });
    // TTL index for shares; harmless to repeat on every start.
    tokio::spawn({
        let state = state.clone();
        async move {
            if let Err(e) = state.shares.ensure_indexes().await {
                tracing::warn!(error = %e, "could not create the shares TTL index");
            }
        }
    });

    // Everything the API offers lives under /api/v1. Breaking changes get /api/v2 alongside.
    let v1 = Router::new()
        .route("/run", post(run))
        .route("/validate", post(validate))
        .route("/explain", post(explain))
        .route("/share", post(share))
        .route("/p/{id}", get(load_share))
        .route("/events", post(ui_event))
        .route("/export", post(export_handler))
        .route("/verify", post(guard::verify))
        .route("/samples", get(samples))
        .route("/samples/{db}/{collection}", get(sample_preview))
        .route("/examples", get(examples_handler))
        .route("/guide", get(guide_handler))
        .route("/limits", get(limits_handler))
        // Agent discovery: OpenAPI and an MCP endpoint (Streamable HTTP, stateless).
        .route("/openapi.json", get(openapi))
        .route("/mcp", post(mcp_handler).get(mcp_get))
        // Unknown API paths are JSON 404s, not the SPA's index.html.
        .fallback(|| async {
            ApiError(PlaygroundError::NotFound(
                "no such endpoint; see /api/v1/openapi.json".into(),
            ))
        });

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(readyz))
        .nest("/api/v1", v1)
        // Scheduled jobs (Cloud Scheduler). Not part of the public API.
        .route("/internal/jobs/sweep", post(job_sweep))
        .route("/internal/jobs/rollup", post(job_rollup))
        // Conventional root-level discovery paths for agents and crawlers.
        .route("/openapi.json", get(openapi))
        .route("/llms.txt", get(llms_txt))
        .route("/.well-known/mcp.json", get(mcp_card))
        .route("/mcp", post(mcp_handler).get(mcp_get));

    // The web app (built by `npm run build` in web/) is served from the same origin, with
    // index.html as the fallback so deep links like /p/<id> open the app. Set WEB_DIR to enable.
    let web_dir = env("WEB_DIR").unwrap_or_else(|| "web/dist".into());
    let app = if std::path::Path::new(&web_dir).join("index.html").exists() {
        tracing::info!(%web_dir, "serving the web app");
        let files =
            ServeDir::new(&web_dir).fallback(ServeFile::new(format!("{web_dir}/index.html")));
        app.fallback_service(
            tower::ServiceBuilder::new()
                .layer(SetResponseHeaderLayer::if_not_present(
                    header::CACHE_CONTROL,
                    axum::http::HeaderValue::from_static("public, max-age=300"),
                ))
                .service(files),
        )
    } else {
        app
    };

    let app = app
        // Charged per client; reads and discovery are free (see guard::cost).
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            guard::limit,
        ))
        .layer(DefaultBodyLimit::max(body_limit))
        // Requests may arrive gzip/br/zstd-compressed (a 32 MB payload compresses well, and
        // Cloud Run caps HTTP/1 request bodies at 32 MiB on the wire); responses are
        // compressed when the client sends Accept-Encoding.
        .layer(RequestDecompressionLayer::new())
        .layer(CompressionLayer::new())
        // The SPA is served from the same origin in production (behind the load balancer);
        // permissive CORS matters for local dev and for browser-based agents.
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    let addr: SocketAddr = format!("{host}:{port}").parse().expect("invalid BIND/PORT");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("cannot bind");
    tracing::info!(%addr, "listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");
    state.stats.flush().await;
}

// ----- request context and stats ----------------------------------------------------------------

/// What we may remember about a caller: coarse client kind, a rotating session id, a salted
/// daily hash of the address and the country Cloudflare reports. Never the address itself.
struct Ctx {
    client: String,
    session_id: Option<String>,
    ip_hash: Option<String>,
    country: Option<String>,
    bytes_in: Option<i64>,
}

fn ctx(state: &AppState, headers: &HeaderMap, agent: bool) -> Ctx {
    let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let client = match (agent, get("x-client")) {
        (true, _) | (_, Some("agent")) => "agent",
        (_, Some("desktop")) => "desktop",
        _ => "web",
    };
    let ip = get("cf-connecting-ip")
        .or_else(|| get("x-forwarded-for").and_then(|v| v.split(',').next()))
        .map(str::trim);
    let day = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0);
    Ctx {
        client: client.to_string(),
        session_id: get("x-session-id").and_then(stats::clean_session_id),
        ip_hash: ip
            .filter(|i| !i.is_empty())
            .map(|i| stats::ip_hash(i, &state.stats_salt, day)),
        country: get("cf-ipcountry")
            .filter(|c| c.len() == 2 && c.chars().all(|ch| ch.is_ascii_alphabetic()))
            .map(str::to_string),
        bytes_in: get("content-length").and_then(|v| v.parse().ok()),
    }
}

impl Ctx {
    fn event(&self, kind: &str) -> stats::Event {
        stats::Event {
            kind: kind.to_string(),
            client: self.client.clone(),
            ok: true,
            session_id: self.session_id.clone(),
            ip_hash: self.ip_hash.clone(),
            country: self.country.clone(),
            bytes_in: self.bytes_in,
            ..Default::default()
        }
    }
}

/// Record one event for a finished operation. `rows` picks a row count out of a success value.
fn record<T>(
    state: &AppState,
    ctx: &Ctx,
    kind: &str,
    body: Option<&RunBody>,
    start: Instant,
    result: &Result<T, PlaygroundError>,
    rows: impl FnOnce(&T) -> Option<i64>,
) {
    let mut ev = ctx.event(kind);
    let (mode, datasource) = match body {
        Some(RunBody::Script(r)) => ("script".to_string(), r.datasource.as_ref()),
        Some(RunBody::Structured(r)) => (r.mode.as_str().to_string(), r.datasource.as_ref()),
        None => (String::new(), None),
    };
    ev.mode = Some(mode).filter(|m| !m.is_empty());
    ev.datasource = datasource.map(|d| match d {
        playground_core::Datasource::Sample(db) => db.clone(),
    });
    ev.dur_ms = Some(start.elapsed().as_millis() as i64);
    match result {
        Ok(v) => ev.rows = rows(v),
        Err(e) => {
            ev.ok = false;
            ev.err_code = Some(e.code().to_string());
        }
    }
    state.stats.record(ev);
}

// ----- handlers -------------------------------------------------------------------------------------

async fn readyz(State(s): State<Shared>) -> Result<&'static str, ApiError> {
    s.runner.ping().await?;
    Ok("ok")
}

async fn run(
    State(s): State<Shared>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<RunBody>,
) -> Result<Json<Reply>, ApiError> {
    let (c, start) = (ctx(&s, &headers, false), Instant::now());
    let result = match s.check_features(&body) {
        Ok(()) => s.runner.run_body(&body).await,
        Err(e) => Err(e),
    };
    record(&s, &c, "run", Some(&body), start, &result, reply_rows);
    Ok(Json(result?))
}

fn reply_rows(r: &Reply) -> Option<i64> {
    Some(match r {
        Reply::Structured(o) => o.results.len(),
        Reply::Script(o) => o.statements.iter().map(|s| s.results.len()).sum(),
    } as i64)
}

/// Plan only: no results, writes are explained but not performed. Defaults to `executionStats`.
async fn explain(
    State(s): State<Shared>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<RunBody>,
) -> Result<Json<Reply>, ApiError> {
    let (c, start) = (ctx(&s, &headers, false), Instant::now());
    let result = match s.check_features(&body) {
        Ok(()) => s.runner.explain_body(&body).await,
        Err(e) => Err(e),
    };
    record(&s, &c, "explain", Some(&body), start, &result, |_| None);
    Ok(Json(result?))
}

/// Check a request without touching the database: same validation as `/api/v1/run`, no side effects.
async fn validate(
    State(s): State<Shared>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<RunBody>,
) -> Result<Json<Value>, ApiError> {
    let (c, start) = (ctx(&s, &headers, false), Instant::now());
    let result = match s.check_features(&body) {
        Ok(()) => s.runner.validate_body(&body),
        Err(e) => Err(e),
    };
    record(&s, &c, "validate", Some(&body), start, &result, |_| None);
    Ok(Json(result?))
}

async fn share(
    State(s): State<Shared>,
    headers: HeaderMap,
    ApiJson(p): ApiJson<SharedPlayground>,
) -> Result<Json<Value>, ApiError> {
    let (c, start) = (ctx(&s, &headers, false), Instant::now());
    let result = if p.datasource.is_null() || s.features.sample_data {
        s.shares.save(&p, s.runner.limits()).await
    } else {
        Err(PlaygroundError::Invalid(
            "sample data is turned off on this deployment".into(),
        ))
    };
    record(&s, &c, "share", None, start, &result, |_| None);
    let id = result?;
    Ok(Json(json!({ "id": id, "url": format!("/p/{id}") })))
}

async fn load_share(
    State(s): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let (c, start) = (ctx(&s, &headers, false), Instant::now());
    let result = s.shares.load(&id).await;
    record(&s, &c, "load_share", None, start, &result, |_| None);
    let loaded = result?;
    let mut body = serde_json::to_value(&loaded.playground).unwrap_or(Value::Null);
    body["id"] = json!(id);
    body["createdAt"] = json!(loaded.created_at_ms);
    // The id is a hash of the content, so a hit never changes.
    Ok((
        [(header::CACHE_CONTROL, "public, max-age=31536000, immutable")],
        Json(body),
    )
        .into_response())
}

/// Interactions that only the browser knows about (page views, formatting, picking an example).
/// Metadata only; see `playground_core::stats` for the privacy rule.
async fn ui_event(
    State(s): State<Shared>,
    headers: HeaderMap,
    ApiJson(v): ApiJson<Value>,
) -> Result<StatusCode, ApiError> {
    let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
    if !stats::UI_EVENT_TYPES.contains(&kind) {
        return Err(ApiError(PlaygroundError::Invalid(format!(
            "unknown event type; allowed: {}",
            stats::UI_EVENT_TYPES.join(", ")
        ))));
    }
    let short = |key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .filter(|t| {
                !t.is_empty()
                    && t.len() <= 64
                    && t.chars()
                        .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            })
            .map(str::to_string)
    };
    let mut ev = ctx(&s, &headers, false).event(kind);
    ev.mode = short("mode");
    ev.datasource = short("datasource");
    ev.example_id = short("exampleId");
    ev.share_id = short("shareId");
    s.stats.record(ev);
    Ok(StatusCode::NO_CONTENT)
}

/// Take the playground out of the site: a mongosh script, or the data as Extended JSON.
async fn export_handler(
    ApiJson(req): ApiJson<playground_core::ExportRequest>,
) -> Result<Json<Value>, ApiError> {
    let out = playground_core::export(&req)?;
    Ok(Json(json!({
        "format": out.format,
        "filename": out.filename,
        "mimeType": out.mime_type,
        "content": out.content,
    })))
}

async fn samples(State(s): State<Shared>) -> Result<Json<Value>, ApiError> {
    if !s.features.sample_data {
        return Ok(Json(
            json!({ "available": false, "disabled": true, "databases": [] }),
        ));
    }
    let names = s.runner.sample_databases().await?;
    let mut databases = Vec::new();
    for name in &names {
        databases
            .push(json!({ "name": name, "collections": s.runner.sample_collections(name).await? }));
    }
    Ok(Json(
        json!({ "available": !names.is_empty(), "databases": databases }),
    ))
}

async fn sample_preview(
    State(s): State<Shared>,
    Path((db, collection)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    if !s.features.sample_data {
        return Err(ApiError(PlaygroundError::NotFound(
            "sample data is turned off on this deployment".into(),
        )));
    }
    let documents = s.runner.sample_preview(&db, &collection).await?;
    Ok(Json(
        json!({ "database": db, "collection": collection, "documents": documents }),
    ))
}

async fn examples_handler(State(s): State<Shared>) -> Json<Value> {
    Json(s.visible_examples())
}

/// How to run it yourself, get a free or ephemeral Atlas cluster, or connect an agent.
async fn guide_handler(State(s): State<Shared>) -> Json<Value> {
    Json(playground_core::guide_for(&s.public_url))
}

/// A discovery card for MCP clients and crawlers.
async fn mcp_card(State(s): State<Shared>) -> Json<Value> {
    let origin = s.public_url.trim_end_matches('/');
    Json(json!({
        "name": "MongoDB Playground",
        "description": "Run mongosh-style scripts (several statements, in order) against documents you supply, in a throwaway database, with query plans, Atlas Search and Vector Search.",
        "transport": "streamable-http",
        "url": format!("{origin}/mcp"),
        "authentication": "none",
        "tools": mcp::TOOL_NAMES,
        "docs": format!("{origin}/llms.txt"),
        "openapi": format!("{origin}/api/v1/openapi.json"),
        "setupGuide": format!("{origin}/api/v1/guide"),
    }))
}

async fn limits_handler(State(s): State<Shared>) -> Json<Value> {
    Json(limits_json(&s).await)
}

pub(crate) async fn limits_json(state: &AppState) -> Value {
    let l = state.runner.limits();
    let sample = state.features.sample_data
        && state
            .runner
            .sample_databases()
            .await
            .map(|d| !d.is_empty())
            .unwrap_or(false);
    json!({
        "apiVersion": "v1",
        "maxCollections": l.max_collections,
        "maxDocsPerCollection": l.max_docs_per_collection,
        "maxPayloadBytes": l.max_payload_bytes,
        "maxTimeMs": l.max_time_ms,
        // There is no cap on the number of results (sample-data reads are capped, see below).
        "maxResults": null,
        "maxIndexesPerCollection": l.max_indexes_per_collection,
        "maxIndexKeyFields": l.max_index_key_fields,
        "maxSearchIndexesPerCollection": 1,
        "maxRegexPatternLength": l.max_regex_pattern_len,
        "maxRegexLiteralsPerEditor": 20,
        "searchSupported": state.runner.search_supported().await,
        "searchWallClockMs": l.search_wall_clock_ms,
        "maxConcurrentSearchRuns": l.max_concurrent_search_runs,
        "sampleSupported": sample,
        "features": { "sampleData": state.features.sample_data },
        "deployment": { "cluster": std::env::var("CLUSTER_TIER").unwrap_or_else(|_| "M10".into()) },
        "sampleMaxResults": l.sample_max_results,
        "maxShareBytes": l.max_share_bytes,
        "rateLimit": {
            "basic": { "burst": state.guard.limiter.config().basic.0, "perSecond": state.guard.limiter.config().basic.1 },
            "verified": { "burst": state.guard.limiter.config().verified.0, "perSecond": state.guard.limiter.config().verified.1 },
            "note": "Per client address. Without Turnstile configured everyone is on the verified tier; with it, a verified browser session gets the verified tier. 429 responses carry Retry-After.",
        },
        "turnstile": { "enabled": state.guard.turnstile_enabled(), "siteKey": state.guard.site_key },
        "shareTtlDays": playground_core::SHARE_TTL_SECS / 86_400,
        "compression": { "request": ["gzip", "br", "zstd", "deflate"], "response": ["gzip", "br", "zstd", "deflate"] },
    })
}

async fn openapi() -> Response {
    (
        [(header::CONTENT_TYPE, "application/json")],
        include_str!("openapi.json"),
    )
        .into_response()
}

async fn llms_txt() -> Response {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        include_str!("llms.txt"),
    )
        .into_response()
}

async fn mcp_handler(
    State(s): State<Shared>,
    headers: HeaderMap,
    Json(msg): Json<Value>,
) -> Response {
    let (c, start) = (ctx(&s, &headers, true), Instant::now());
    match mcp::handle(&s, msg).await {
        Some(reply) => {
            let mut ev = c.event("mcp_call");
            ev.dur_ms = Some(start.elapsed().as_millis() as i64);
            ev.ok = reply.get("error").is_none() && reply["result"]["isError"] != json!(true);
            s.stats.record(ev);
            Json(reply).into_response()
        }
        // Notifications get no body.
        None => StatusCode::ACCEPTED.into_response(),
    }
}

async fn mcp_get() -> Response {
    // This server does not stream; POST JSON-RPC messages to this URL.
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST")]).into_response()
}

// ----- scheduled jobs -------------------------------------------------------------------------

fn authorised(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let given = headers
        .get("x-internal-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    match &state.internal_token {
        Some(t) if !t.is_empty() && t == given => Ok(()),
        // 404, not 401: the endpoints do not exist unless configured.
        _ => Err(ApiError(PlaygroundError::NotFound("not found".into()))),
    }
}

/// Drop `pg_*` databases left behind by a crash (older than 10 minutes).
async fn job_sweep(State(s): State<Shared>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    authorised(&s, &headers)?;
    let dropped = s.runner.sweep_stale(Duration::from_secs(600)).await?;
    Ok(Json(json!({ "dropped": dropped })))
}

/// Fold the last 48 hours of events into `daily_rollups`.
async fn job_rollup(State(s): State<Shared>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    authorised(&s, &headers)?;
    s.stats.flush().await;
    stats::rollup(&s.stats_client, &s.stats_db, 48)
        .await
        .map_err(PlaygroundError::from)?;
    Ok(Json(json!({ "ok": true })))
}

// ----- errors -----------------------------------------------------------------------------------

/// The structured error every endpoint returns: `{code, message, hint}`.
pub(crate) fn error_json(e: &PlaygroundError) -> Value {
    json!({ "code": e.code(), "message": e.to_string(), "hint": e.hint() })
}

/// `Json`, but a malformed body (bad JSON, bad regex literal, wrong field type) comes back as
/// our structured `{code, message, hint}` error instead of axum's plain-text rejection.
struct ApiJson<T>(T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: serde::de::DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(Self(value)),
            Err(rejection) => Err(ApiError(PlaygroundError::Invalid(rejection.body_text()))),
        }
    }
}

pub(crate) struct ApiError(pub(crate) PlaygroundError);

impl From<PlaygroundError> for ApiError {
    fn from(e: PlaygroundError) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            PlaygroundError::Invalid(_) | PlaygroundError::Database(_) => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            PlaygroundError::Forbidden(_) => StatusCode::FORBIDDEN,
            PlaygroundError::Timeout(_) => StatusCode::REQUEST_TIMEOUT,
            PlaygroundError::NotFound(_) => StatusCode::NOT_FOUND,
            PlaygroundError::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            PlaygroundError::Unavailable | PlaygroundError::Busy => StatusCode::SERVICE_UNAVAILABLE,
        };
        (status, Json(json!({ "error": error_json(&self.0) }))).into_response()
    }
}

/// A random session secret for single-instance runs.
fn uuid_secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
