use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use mongodb::{
    bson::{doc, Bson, Document},
    options::{ClientOptions, Compressor, ServerApi, ServerApiVersion},
    Client, Database,
};
use serde::Serialize;
use tokio::sync::{OnceCell, RwLock, Semaphore};

use crate::{
    error::PlaygroundError,
    explain::{scrub, summarize, ExplainOutput},
    limits::Limits,
    request::{RunBody, RunRequest, ScriptRequest, Verbosity},
    shell::{prepare_script, Op, PreparedScript, Statement},
    validate::{prepare, Prepared, Query},
};

type Result<T> = std::result::Result<T, PlaygroundError>;

const SAMPLE_LIST_TTL: Duration = Duration::from_secs(600);

/// Counts reported by write modes. Only the fields that apply are present.
#[derive(Debug, Default, Serialize)]
pub struct WriteStats {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inserted: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upserted: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct RunOutput {
    /// Relaxed Extended JSON. Reads: the matching documents (`count` returns `[{"count": n}]`,
    /// `distinct` the values). Writes: the collection contents after the write.
    /// Empty for explain-only runs.
    pub results: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write: Option<WriteStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explain: Option<ExplainOutput>,
    /// Set on sample-data reads that hit the result cap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    pub elapsed_ms: u64,
}

#[derive(Clone, Copy)]
enum Action {
    Run,
    ExplainOnly(Verbosity),
}

struct Inner {
    results: Vec<Bson>,
    write: Option<WriteStats>,
    explain: Option<ExplainOutput>,
    truncated: Option<bool>,
}

/// Runs a request against a MongoDB deployment inside a throwaway database.
pub struct Runner {
    client: Client,
    sample_client: Client,
    limits: Limits,
    search_permits: Arc<Semaphore>,
    search_supported: OnceCell<bool>,
    sample_dbs: RwLock<Option<(Instant, Vec<String>)>>,
}

impl Runner {
    /// Build the shared client(s). The driver connects lazily, so this succeeds even when
    /// MongoDB is not up yet. See [`make_client`] for the connection defaults.
    pub async fn connect(uri: &str, limits: Limits) -> Result<Self> {
        Self::connect_with(uri, None, limits).await
    }

    /// `sample_uri` should use a read-only user that can read the `sample_*` databases; without
    /// it the main client is used for sample-data reads.
    pub async fn connect_with(uri: &str, sample_uri: Option<&str>, limits: Limits) -> Result<Self> {
        let client = make_client(uri).await?;
        let sample_client = match sample_uri {
            Some(u) => make_client(u).await?,
            None => client.clone(),
        };
        let search_permits = Arc::new(Semaphore::new(limits.max_concurrent_search_runs));
        Ok(Self {
            client,
            sample_client,
            limits,
            search_permits,
            search_supported: OnceCell::new(),
            sample_dbs: RwLock::new(None),
        })
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// The shared client, for components that use their own database (shares, stats).
    pub fn client(&self) -> &Client {
        &self.client
    }

    pub async fn ping(&self) -> Result<()> {
        self.client
            .database("admin")
            .run_command(doc! { "ping": 1 })
            .await?;
        Ok(())
    }

    /// Whether this deployment can build Atlas Search indexes (Atlas, or `atlas local`).
    /// Only a definite answer is cached; a connection failure is retried next time.
    pub async fn search_supported(&self) -> bool {
        self.search_supported
            .get_or_try_init(|| async {
                let db = self.client.database(&new_db_name("pg_probe"));
                let outcome = async {
                    db.run_command(doc! { "create": "probe" }).await?;
                    db.run_command(doc! {
                        "aggregate": "probe",
                        "pipeline": [ { "$listSearchIndexes": {} } ],
                        "cursor": {},
                    })
                    .await
                }
                .await;
                let _ = db.run_command(doc! { "dropDatabase": 1 }).await;
                match outcome {
                    Ok(_) => Ok(true),
                    Err(e) if matches!(&*e.kind, mongodb::error::ErrorKind::Command(_)) => {
                        Ok(false)
                    }
                    Err(_) => Err(()),
                }
            })
            .await
            .copied()
            .unwrap_or(false)
    }

    /// Run the request. If `req.explain` is set, the plan is returned alongside the results.
    pub async fn run(&self, req: &RunRequest) -> Result<RunOutput> {
        let prepared = prepare(req, &self.limits)?;
        self.sandboxed(prepared, Action::Run).await
    }

    /// Explain only: the plan, no results. Defaults to `executionStats` when `req.explain`
    /// is not set. Writes are explained, not performed.
    pub async fn explain(&self, req: &RunRequest) -> Result<RunOutput> {
        let mut req = req.clone();
        let verbosity = *req.explain.get_or_insert(Verbosity::ExecutionStats);
        let prepared = prepare(&req, &self.limits)?;
        self.sandboxed(prepared, Action::ExplainOnly(verbosity))
            .await
    }

    /// Check a request without touching the database.
    pub fn validate(&self, req: &RunRequest) -> Result<Prepared> {
        prepare(req, &self.limits)
    }

    async fn sandboxed(&self, prepared: Prepared, action: Action) -> Result<RunOutput> {
        if prepared.sample_db.is_some() {
            return self.run_sample(prepared, action).await;
        }

        let (_permit, budget_ms) = self
            .search_gate(!prepared.search_indexes.is_empty())
            .await?;
        let db_name = new_db_name("pg");
        let db = self.client.database(&db_name);
        let start = Instant::now();

        let outcome = tokio::time::timeout(
            Duration::from_millis(budget_ms),
            self.execute(&db, &prepared, action),
        )
        .await;

        // Always clean up, including after a timeout or an error. The sweep job removes
        // `pg_*` databases that survive a crash between here and the process exit.
        if let Err(e) = db.run_command(doc! { "dropDatabase": 1 }).await {
            tracing::warn!(db = %db_name, error = %e, "failed to drop playground database");
        }

        match outcome {
            Ok(inner) => Ok(output(inner?, start)),
            Err(_) => Err(PlaygroundError::Timeout(budget_ms)),
        }
    }

    async fn execute(&self, db: &Database, p: &Prepared, action: Action) -> Result<Inner> {
        self.seed(db, &p.collections, &p.indexes, &p.search_indexes)
            .await?;

        // Explain before running: explaining a write reports the plan without performing it.
        let verbosity = match action {
            Action::ExplainOnly(v) => Some(v),
            Action::Run => p.explain,
        };
        let explain = match verbosity {
            Some(v) => Some(self.explain_op(db, p, v).await?),
            None => None,
        };
        if matches!(action, Action::ExplainOnly(_)) {
            return Ok(Inner {
                results: vec![],
                write: None,
                explain,
                truncated: None,
            });
        }

        let (results, write) = self.run_op(db, p).await?;
        Ok(Inner {
            results,
            write,
            explain,
            truncated: None,
        })
    }

    async fn explain_op(
        &self,
        db: &Database,
        p: &Prepared,
        verbosity: Verbosity,
    ) -> Result<ExplainOutput> {
        if matches!(p.query, Query::Insert { .. } | Query::ListIndexes) {
            return Err(PlaygroundError::Invalid(
                "this mode cannot be explained".into(),
            ));
        }
        let inner = command(p, self.limits.max_time_ms as i64);
        let res = db
            .run_command(doc! { "explain": inner, "verbosity": verbosity.as_str() })
            .await?;
        let mut raw = Bson::Document(res).into_relaxed_extjson();
        if let Some(obj) = raw.as_object_mut() {
            obj.remove("ok");
        }
        scrub(&mut raw);
        let summary = summarize(&raw, verbosity);
        Ok(ExplainOutput { raw, summary })
    }

    async fn run_op(&self, db: &Database, p: &Prepared) -> Result<(Vec<Bson>, Option<WriteStats>)> {
        let cmd = command(p, self.limits.max_time_ms as i64);
        if matches!(
            &p.query,
            Query::Find { .. } | Query::Aggregate { .. } | Query::ListIndexes
        ) {
            return Ok((
                run_cursor(db, cmd, self.limits.max_result_bytes).await?,
                None,
            ));
        }
        let res = db.run_command(cmd).await?;
        match &p.query {
            Query::Find { .. } | Query::Aggregate { .. } | Query::ListIndexes => {
                unreachable!("cursor queries return above")
            }
            Query::Count { .. } => Ok((
                vec![Bson::Document(doc! { "count": number(&res, "n") })],
                None,
            )),
            Query::Distinct { .. } => {
                let values = res.get_array("values").map_err(|_| {
                    PlaygroundError::Database("unexpected response from MongoDB".into())
                })?;
                Ok((values.to_vec(), None))
            }
            Query::Insert { .. } => {
                check_write_errors(&res)?;
                let stats = WriteStats {
                    inserted: Some(number(&res, "n")),
                    ..Default::default()
                };
                Ok((self.all_documents(db, &p.collection).await?, Some(stats)))
            }
            Query::Update { .. } => {
                check_write_errors(&res)?;
                let upserted = res
                    .get_array("upserted")
                    .map(|a| a.len() as i64)
                    .unwrap_or(0);
                let stats = WriteStats {
                    // `n` counts matched and upserted documents together.
                    matched: Some(number(&res, "n") - upserted),
                    modified: Some(number(&res, "nModified")),
                    upserted: Some(upserted),
                    ..Default::default()
                };
                Ok((self.all_documents(db, &p.collection).await?, Some(stats)))
            }
            Query::Delete { .. } => {
                check_write_errors(&res)?;
                let stats = WriteStats {
                    deleted: Some(number(&res, "n")),
                    ..Default::default()
                };
                Ok((self.all_documents(db, &p.collection).await?, Some(stats)))
            }
        }
    }

    async fn all_documents(&self, db: &Database, collection: &str) -> Result<Vec<Bson>> {
        run_cursor(
            db,
            doc! {
                "find": collection,
                "filter": {},
                "maxTimeMS": self.limits.max_time_ms as i64,
            },
            self.limits.max_result_bytes,
        )
        .await
    }

    // ----- sample data ------------------------------------------------------------------

    /// Names of the `sample_*` databases on the sample client (cached for ten minutes).
    pub async fn sample_databases(&self) -> Result<Vec<String>> {
        if let Some((at, names)) = &*self.sample_dbs.read().await {
            if at.elapsed() < SAMPLE_LIST_TTL {
                return Ok(names.clone());
            }
        }
        let res = self
            .sample_client
            .database("admin")
            .run_command(doc! { "listDatabases": 1, "nameOnly": true })
            .await?;
        let mut names: Vec<String> = res
            .get_array("databases")
            .map(|a| {
                a.iter()
                    .filter_map(|d| d.as_document().and_then(|d| d.get_str("name").ok()))
                    .filter(|n| n.starts_with("sample_"))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        *self.sample_dbs.write().await = Some((Instant::now(), names.clone()));
        Ok(names)
    }

    /// Collections of a sample database.
    pub async fn sample_collections(&self, database: &str) -> Result<Vec<String>> {
        self.require_sample_db(database).await?;
        let res = self
            .sample_client
            .database(database)
            .run_command(doc! { "listCollections": 1, "nameOnly": true })
            .await?;
        let mut names: Vec<String> = res
            .get_document("cursor")
            .and_then(|c| c.get_array("firstBatch"))
            .map(|a| {
                a.iter()
                    .filter_map(|d| d.as_document().and_then(|d| d.get_str("name").ok()))
                    .filter(|n| !n.starts_with("system."))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        Ok(names)
    }

    /// Three random documents from a sample collection, so people can see the field names.
    pub async fn sample_preview(
        &self,
        database: &str,
        collection: &str,
    ) -> Result<Vec<serde_json::Value>> {
        self.require_sample_db(database).await?;
        if !self
            .sample_collections(database)
            .await?
            .iter()
            .any(|c| c == collection)
        {
            return Err(PlaygroundError::NotFound(format!(
                "no collection `{collection}` in `{database}`"
            )));
        }
        let res = self
            .sample_client
            .database(database)
            .run_command(doc! {
                "aggregate": collection,
                "pipeline": [ { "$sample": { "size": 3 } } ],
                "cursor": {},
                "maxTimeMS": self.limits.max_time_ms as i64,
            })
            .await?;
        let batch = res
            .get_document("cursor")
            .and_then(|c| c.get_array("firstBatch"))
            .map(|a| a.to_vec())
            .unwrap_or_default();
        Ok(batch.into_iter().map(Bson::into_relaxed_extjson).collect())
    }

    async fn require_sample_db(&self, name: &str) -> Result<()> {
        let names = self.sample_databases().await?;
        if names.iter().any(|n| n == name) {
            Ok(())
        } else {
            Err(PlaygroundError::Invalid(format!(
                "unknown sample database `{name}`; available: {}",
                if names.is_empty() {
                    "none (sample data is not loaded here)".to_string()
                } else {
                    names.join(", ")
                }
            )))
        }
    }

    async fn run_sample(&self, p: Prepared, action: Action) -> Result<RunOutput> {
        let name = p.sample_db.clone().unwrap_or_default();
        self.require_sample_db(&name).await?;
        let db = self.sample_client.database(&name);
        let start = Instant::now();
        // Never drop or write: this is the shared, read-only sample database.
        let inner = tokio::time::timeout(
            Duration::from_millis(self.limits.wall_clock_ms),
            self.sample_exec(&db, &p, action),
        )
        .await
        .map_err(|_| PlaygroundError::Timeout(self.limits.wall_clock_ms))??;
        Ok(output(inner, start))
    }

    async fn sample_exec(&self, db: &Database, p: &Prepared, action: Action) -> Result<Inner> {
        let verbosity = match action {
            Action::ExplainOnly(v) => Some(v),
            Action::Run => p.explain,
        };
        let explain = match verbosity {
            Some(v) => Some(self.explain_op(db, p, v).await?),
            None => None,
        };
        if matches!(action, Action::ExplainOnly(_)) {
            return Ok(Inner {
                results: vec![],
                write: None,
                explain,
                truncated: None,
            });
        }
        let cap = self.limits.sample_max_results;
        let (mut results, _) = self.run_op(db, &capped(p, cap)).await?;
        let truncated = results.len() as i64 > cap;
        results.truncate(cap as usize);
        Ok(Inner {
            results,
            write: None,
            explain,
            truncated: truncated.then_some(true),
        })
    }

    // ----- jobs -------------------------------------------------------------------------

    /// Drop leftover `pg_*` databases older than `older_than` (a crash between seeding and
    /// cleanup). Returns the names dropped. Run on a schedule.
    pub async fn sweep_stale(&self, older_than: Duration) -> Result<Vec<String>> {
        let res = self
            .client
            .database("admin")
            .run_command(doc! { "listDatabases": 1, "nameOnly": true })
            .await?;
        let now = now_secs();
        let mut dropped = Vec::new();
        for d in res
            .get_array("databases")
            .map(|a| a.to_vec())
            .unwrap_or_default()
        {
            let Some(name) = d.as_document().and_then(|d| d.get_str("name").ok()) else {
                continue;
            };
            // Names are `pg_<unix seconds in hex>_<random>`.
            let Some(created) = name
                .strip_prefix("pg_")
                .and_then(|rest| rest.split('_').next())
                .and_then(|hex| u64::from_str_radix(hex, 16).ok())
            else {
                continue;
            };
            if now.saturating_sub(created) >= older_than.as_secs() {
                self.client
                    .database(name)
                    .run_command(doc! { "dropDatabase": 1 })
                    .await?;
                dropped.push(name.to_string());
            }
        }
        Ok(dropped)
    }
}

fn output(inner: Inner, start: Instant) -> RunOutput {
    RunOutput {
        results: inner
            .results
            .into_iter()
            .map(Bson::into_relaxed_extjson)
            .collect(),
        write: inner.write,
        explain: inner.explain,
        truncated: inner.truncated,
        elapsed_ms: start.elapsed().as_millis() as u64,
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `<prefix>_<unix seconds hex>_<random>`; the timestamp lets the sweep job find stale ones.
fn new_db_name(prefix: &str) -> String {
    let id = uuid::Uuid::new_v4().simple().to_string();
    format!("{prefix}_{:x}_{}", now_secs(), &id[..12])
}

/// Sample reads are capped: ask for one more than the cap so truncation can be detected.
fn capped(p: &Prepared, cap: i64) -> Prepared {
    let mut p = p.clone();
    match &mut p.query {
        Query::Find { limit, .. } => *limit = Some(limit.map_or(cap + 1, |l| l.min(cap + 1))),
        Query::Aggregate { pipeline } => pipeline.push(doc! { "$limit": cap + 1 }),
        _ => {}
    }
    p
}

/// Build the shared client with connection best practices, applied only where the connection
/// string left a value unset (so operators can tune it in the URI without a rebuild).
pub async fn make_client(uri: &str) -> Result<Client> {
    let mut opts = ClientOptions::parse(uri).await?;
    apply_defaults(&mut opts);
    Ok(Client::with_options(opts)?)
}

/// The MongoDB command for the prepared query. Used both to run it and to explain it.
fn command(p: &Prepared, max_time: i64) -> Document {
    let coll = &p.collection;
    match &p.query {
        Query::Find {
            filter,
            projection,
            sort,
            limit,
            skip,
        } => {
            let mut cmd = doc! { "find": coll, "filter": filter.clone(), "maxTimeMS": max_time };
            if let Some(n) = limit {
                cmd.insert("limit", *n);
            }
            if let Some(n) = skip {
                cmd.insert("skip", *n);
            }
            if let Some(pr) = projection {
                cmd.insert("projection", pr.clone());
            }
            if let Some(s) = sort {
                cmd.insert("sort", s.clone());
            }
            cmd
        }
        Query::Aggregate { pipeline } => {
            let stages: Vec<Bson> = pipeline.iter().cloned().map(Bson::Document).collect();
            doc! { "aggregate": coll, "pipeline": stages, "cursor": {}, "maxTimeMS": max_time }
        }
        Query::Count { filter } => {
            doc! { "count": coll, "query": filter.clone(), "maxTimeMS": max_time }
        }
        Query::Distinct { key, filter } => {
            doc! { "distinct": coll, "key": key, "query": filter.clone(), "maxTimeMS": max_time }
        }
        Query::Insert { documents } => {
            let docs: Vec<Bson> = documents.iter().cloned().map(Bson::Document).collect();
            doc! { "insert": coll, "documents": docs }
        }
        Query::Update {
            filter,
            update,
            multi,
            upsert,
        } => doc! {
            "update": coll,
            "updates": [ { "q": filter.clone(), "u": update.clone(), "multi": *multi, "upsert": *upsert } ],
            "maxTimeMS": max_time,
        },
        Query::ListIndexes => doc! { "listIndexes": coll },
        Query::Delete { filter, multi } => doc! {
            "delete": coll,
            "deletes": [ { "q": filter.clone(), "limit": if *multi { 0 } else { 1 } } ],
            "maxTimeMS": max_time,
        },
    }
}

/// Write commands report per-document failures in `writeErrors` instead of failing the command.
fn check_write_errors(res: &Document) -> Result<()> {
    if let Ok(errs) = res.get_array("writeErrors") {
        if let Some(Bson::Document(e)) = errs.first() {
            let msg = e.get_str("errmsg").unwrap_or("write failed");
            return Err(PlaygroundError::Database(msg.to_string()));
        }
    }
    Ok(())
}

/// Create each declared search index, then wait until it is queryable. The overall run budget
/// (`search_wall_clock_ms`) bounds the wait.
async fn create_search_indexes(
    db: &Database,
    search: &std::collections::BTreeMap<String, Vec<Document>>,
) -> Result<()> {
    for (coll, specs) in search {
        let specs: Vec<Bson> = specs.iter().cloned().map(Bson::Document).collect();
        db.run_command(doc! { "createSearchIndexes": coll, "indexes": specs })
            .await?;
    }
    for (coll, specs) in search {
        for spec in specs {
            wait_for_search_index(db, coll, spec.get_str("name").unwrap_or("default")).await?;
        }
    }
    Ok(())
}

/// Create one search or vector search index and wait until it is queryable.
async fn create_search_index(db: &Database, coll: &str, spec: &Document) -> Result<()> {
    db.run_command(doc! { "createSearchIndexes": coll, "indexes": [ spec.clone() ] })
        .await?;
    wait_for_search_index(db, coll, spec.get_str("name").unwrap_or("default")).await
}

/// Poll until the index is queryable; the caller's overall time budget bounds the wait.
async fn wait_for_search_index(db: &Database, coll: &str, name: &str) -> Result<()> {
    loop {
        let res = db
            .run_command(doc! {
                "aggregate": coll,
                "pipeline": [ { "$listSearchIndexes": { "name": name } } ],
                "cursor": {},
            })
            .await?;
        let first = res
            .get_document("cursor")
            .and_then(|c| c.get_array("firstBatch"))
            .ok()
            .and_then(|b| b.first())
            .and_then(|d| d.as_document());
        if let Some(d) = first {
            if d.get_str("status") == Ok("FAILED") {
                return Err(PlaygroundError::Database(format!(
                    "the search index `{name}` failed to build; check its definition"
                )));
            }
            if d.get_bool("queryable").unwrap_or(false) {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Connection best practices, applied only where the connection string left a value unset.
fn apply_defaults(opts: &mut ClientOptions) {
    // Wire compression: the server picks the first mutually supported one. Atlas supports all
    // three. Results and seed documents are repetitive JSON-like BSON, so this is cheap and helps.
    if opts.compressors.is_none() {
        opts.compressors = Some(vec![
            Compressor::Zstd { level: None },
            Compressor::Snappy,
            Compressor::Zlib { level: None },
        ]);
    }
    // Serverless instance: each run holds one connection at a time, so the pool only needs to
    // match the per-instance request concurrency (set Cloud Run concurrency to the same value).
    opts.max_pool_size.get_or_insert(5);
    // Do not keep connections warm: scale-to-zero instances should not hold idle server slots.
    opts.min_pool_size.get_or_insert(0);
    opts.max_idle_time.get_or_insert(Duration::from_secs(30));
    opts.connect_timeout.get_or_insert(Duration::from_secs(10));
    opts.server_selection_timeout
        .get_or_insert(Duration::from_secs(5));
    opts.retry_reads.get_or_insert(true);
    opts.retry_writes.get_or_insert(true);
    opts.app_name
        .get_or_insert_with(|| "mongodb-playground".to_string());
    // Stable API v1: behaviour stays the same across server upgrades. Not strict, so
    // playground users can still try any operator.
    if opts.server_api.is_none() {
        opts.server_api = Some(ServerApi::builder().version(ServerApiVersion::V1).build());
    }
}

/// Run a cursor-returning command and collect every batch. A cursor belongs to the session
/// that created it, and raw commands otherwise take a pooled implicit session each time, so
/// under concurrency `getMore` could land on a different session. One explicit session for
/// the command and all its `getMore`s keeps them together.
async fn run_cursor(db: &Database, cmd: Document, max_bytes: usize) -> Result<Vec<Bson>> {
    let mut session = db.client().start_session().await?;
    let reply = db.run_command(cmd).session(&mut session).await?;
    drain(db, &mut session, &reply, max_bytes).await
}

/// Collect every batch of a cursor reply: the first batch plus `getMore` until exhausted.
async fn drain(
    db: &Database,
    session: &mut mongodb::ClientSession,
    reply: &Document,
    max_bytes: usize,
) -> Result<Vec<Bson>> {
    let bad = || PlaygroundError::Database("unexpected response from MongoDB".into());
    let cursor = reply.get_document("cursor").map_err(|_| bad())?;
    let mut out: Vec<Bson> = cursor.get_array("firstBatch").map_err(|_| bad())?.to_vec();
    let mut id = cursor_id(cursor);
    let mut bytes = approx_size(&out);
    let too_big = || {
        PlaygroundError::Database(format!(
            "the results are larger than {} MB; narrow them with a filter, $project or $limit",
            max_bytes / (1024 * 1024)
        ))
    };
    if bytes > max_bytes {
        return Err(too_big());
    }
    // `ns` is "<db>.<collection>"; getMore needs the collection part.
    let coll = cursor
        .get_str("ns")
        .ok()
        .and_then(|ns| ns.split_once('.').map(|(_, c)| c.to_string()))
        .ok_or_else(bad)?;

    while id != 0 {
        let next = db
            .run_command(doc! { "getMore": id, "collection": &coll })
            .session(&mut *session)
            .await?;
        let c = next.get_document("cursor").map_err(|_| bad())?;
        let batch = c.get_array("nextBatch").map_err(|_| bad())?;
        bytes += approx_size(batch);
        if bytes > max_bytes {
            // Stop pulling: the cursor dies with the throwaway database.
            return Err(too_big());
        }
        out.extend(batch.iter().cloned());
        id = cursor_id(c);
    }
    Ok(out)
}

/// Rough size of a batch in bytes, from the BSON encoding.
fn approx_size(batch: &[Bson]) -> usize {
    batch
        .iter()
        .map(|b| match b {
            Bson::Document(d) => mongodb::bson::to_vec(d).map(|v| v.len()).unwrap_or(0),
            _ => 16,
        })
        .sum()
}

fn cursor_id(cursor: &Document) -> i64 {
    match cursor.get("id") {
        Some(Bson::Int64(n)) => *n,
        Some(Bson::Int32(n)) => i64::from(*n),
        _ => 0,
    }
}

fn number(doc: &Document, key: &str) -> i64 {
    match doc.get(key) {
        Some(Bson::Int32(n)) => i64::from(*n),
        Some(Bson::Int64(n)) => *n,
        Some(Bson::Double(n)) => *n as i64,
        _ => 0,
    }
}

// ----- shell-style scripts ----------------------------------------------------------------------

/// A failure, as the API reports it.
#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    pub hint: String,
}

impl From<&PlaygroundError> for ErrorBody {
    fn from(e: &PlaygroundError) -> Self {
        Self {
            code: e.code().into(),
            message: e.to_string(),
            hint: e.hint().into(),
        }
    }
}

/// What one statement did: the shell transcript is these, in order.
#[derive(Debug, Serialize)]
pub struct StatementOutput {
    pub index: usize,
    pub line: usize,
    /// The statement as typed.
    pub statement: String,
    pub collection: String,
    /// `find`, `aggregate`, `insert`, `createIndex`, `explain`, ...
    pub op: &'static str,
    pub results: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write: Option<WriteStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explain: Option<ExplainOutput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Serialize)]
pub struct ScriptOutput {
    pub statements: Vec<StatementOutput>,
    /// False when a statement failed: execution stops there, earlier statements keep their output.
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorBody>,
    pub elapsed_ms: u64,
}

/// The reply to `run`/`explain`: a script transcript, or the older single-operation shape.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum Reply {
    Script(ScriptOutput),
    Structured(Box<RunOutput>),
}

impl Runner {
    /// Wait for (and hold) a search slot if the run builds a search or vector index. Returns the
    /// permit and the whole-run time budget.
    async fn search_gate(
        &self,
        uses_search: bool,
    ) -> Result<(Option<tokio::sync::OwnedSemaphorePermit>, u64)> {
        if !uses_search {
            return Ok((None, self.limits.wall_clock_ms));
        }
        if !self.search_supported().await {
            return Err(PlaygroundError::Invalid(
                "this deployment does not support Atlas Search; use Atlas or `atlas local`".into(),
            ));
        }
        match tokio::time::timeout(
            Duration::from_secs(5),
            self.search_permits.clone().acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => Ok((Some(permit), self.limits.search_wall_clock_ms)),
            _ => Err(PlaygroundError::Busy),
        }
    }

    /// Create the collections, load their documents, then build the declared indexes.
    async fn seed(
        &self,
        db: &Database,
        collections: &std::collections::BTreeMap<String, Vec<Document>>,
        indexes: &std::collections::BTreeMap<String, Vec<Document>>,
        search: &std::collections::BTreeMap<String, Vec<Document>>,
    ) -> Result<()> {
        for (name, docs) in collections {
            if docs.is_empty() {
                db.run_command(doc! { "create": name }).await?;
            } else {
                let documents: Vec<Bson> = docs.iter().cloned().map(Bson::Document).collect();
                let res = db
                    .run_command(doc! { "insert": name, "documents": documents })
                    .await?;
                check_write_errors(&res)?;
            }
        }
        // Indexes go in after the data, before the query, so explain can show them being used.
        for (name, specs) in indexes {
            let specs: Vec<Bson> = specs.iter().cloned().map(Bson::Document).collect();
            db.run_command(doc! {
                "createIndexes": name,
                "indexes": specs,
                "maxTimeMS": self.limits.max_time_ms as i64,
            })
            .await?;
        }
        if !search.is_empty() {
            create_search_indexes(db, search).await?;
        }
        Ok(())
    }

    pub async fn run_body(&self, body: &RunBody) -> Result<Reply> {
        match body {
            RunBody::Script(req) => Ok(Reply::Script(self.run_script(req).await?)),
            RunBody::Structured(req) => Ok(Reply::Structured(Box::new(self.run(req).await?))),
        }
    }

    /// Explain only. Scripts: the statements are explained, not run (inserts and index builds
    /// still run, so the plans have data and indexes to work with).
    pub async fn explain_body(&self, body: &RunBody) -> Result<Reply> {
        match body {
            RunBody::Script(req) => {
                let mut req = req.clone();
                req.explain.get_or_insert(Verbosity::ExecutionStats);
                Ok(Reply::Script(self.run_script(&req).await?))
            }
            RunBody::Structured(req) => Ok(Reply::Structured(Box::new(self.explain(req).await?))),
        }
    }

    /// Check a request without touching the database.
    pub fn validate_body(&self, body: &RunBody) -> Result<serde_json::Value> {
        match body {
            RunBody::Structured(req) => {
                Ok(crate::validate::prepare_ok_summary(&self.validate(req)?))
            }
            RunBody::Script(req) => {
                let p = prepare_script(req, &self.limits)?;
                let statements: Vec<_> = p
                    .statements
                    .iter()
                    .map(|s| serde_json::json!({ "index": s.index, "line": s.line, "collection": s.collection, "op": s.op.label() }))
                    .collect();
                let docs: serde_json::Map<String, serde_json::Value> = p
                    .collections
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::from(v.len())))
                    .collect();
                Ok(serde_json::json!({
                    "valid": true,
                    "statements": statements,
                    "documentCounts": docs,
                    "indexes": p.indexes.values().map(Vec::len).sum::<usize>(),
                    "searchIndexes": p.search_indexes.values().map(Vec::len).sum::<usize>(),
                    "sampleDatabase": p.sample_db,
                }))
            }
        }
    }

    pub async fn run_script(&self, req: &ScriptRequest) -> Result<ScriptOutput> {
        let p = prepare_script(req, &self.limits)?;
        if p.sample_db.is_some() {
            return self.script_sample(p).await;
        }
        let (_permit, budget_ms) = self.search_gate(p.uses_search()).await?;
        let db_name = new_db_name("pg");
        let db = self.client.database(&db_name);
        let start = Instant::now();
        let mut outputs = Vec::new();

        let outcome = tokio::time::timeout(
            Duration::from_millis(budget_ms),
            self.script_steps(&db, &p, &mut outputs),
        )
        .await;

        if let Err(e) = db.run_command(doc! { "dropDatabase": 1 }).await {
            tracing::warn!(db = %db_name, error = %e, "failed to drop playground database");
        }
        self.script_output(outcome, outputs, budget_ms, start)
    }

    async fn script_sample(&self, p: PreparedScript) -> Result<ScriptOutput> {
        let name = p.sample_db.clone().unwrap_or_default();
        self.require_sample_db(&name).await?;
        let db = self.sample_client.database(&name);
        let start = Instant::now();
        let mut outputs = Vec::new();
        let budget = self.limits.wall_clock_ms;
        // Never seeds, writes or drops: this is the shared, read-only sample database.
        let outcome = tokio::time::timeout(
            Duration::from_millis(budget),
            self.statements(&db, &p, &mut outputs, Some(self.limits.sample_max_results)),
        )
        .await;
        self.script_output(outcome, outputs, budget, start)
    }

    async fn script_steps(
        &self,
        db: &Database,
        p: &PreparedScript,
        outputs: &mut Vec<StatementOutput>,
    ) -> Result<()> {
        self.seed(db, &p.collections, &p.indexes, &p.search_indexes)
            .await?;
        self.statements(db, p, outputs, None).await
    }

    /// Run the statements in order. The first failure is recorded on its statement and stops the run.
    async fn statements(
        &self,
        db: &Database,
        p: &PreparedScript,
        outputs: &mut Vec<StatementOutput>,
        sample_cap: Option<i64>,
    ) -> Result<()> {
        for stmt in &p.statements {
            let start = Instant::now();
            let result = self
                .exec_statement(db, stmt, p.explain_all, sample_cap)
                .await;
            let failed = result.is_err();
            outputs.push(statement_output(stmt, result, start));
            if failed {
                break;
            }
        }
        Ok(())
    }

    async fn exec_statement(
        &self,
        db: &Database,
        stmt: &Statement,
        explain_all: Option<Verbosity>,
        sample_cap: Option<i64>,
    ) -> Result<Inner> {
        let max_time = self.limits.max_time_ms as i64;
        let empty = |results: Vec<Bson>| Inner {
            results,
            write: None,
            explain: None,
            truncated: None,
        };
        match &stmt.op {
            Op::Query(q) => {
                let p = statement_prepared(&stmt.collection, q.clone());
                if let (Some(v), true) = (
                    explain_all,
                    !matches!(q, Query::Insert { .. } | Query::ListIndexes),
                ) {
                    let explain = Some(self.explain_op(db, &p, v).await?);
                    return Ok(Inner {
                        results: vec![],
                        write: None,
                        explain,
                        truncated: None,
                    });
                }
                match sample_cap {
                    Some(cap) => {
                        let (mut results, _) = self.run_op(db, &capped(&p, cap)).await?;
                        let truncated = results.len() as i64 > cap;
                        results.truncate(cap as usize);
                        Ok(Inner {
                            results,
                            write: None,
                            explain: None,
                            truncated: truncated.then_some(true),
                        })
                    }
                    None => {
                        let (results, write) = self.run_op(db, &p).await?;
                        Ok(Inner {
                            results,
                            write,
                            explain: None,
                            truncated: None,
                        })
                    }
                }
            }
            Op::Explain(v, q) => {
                let p = statement_prepared(&stmt.collection, q.clone());
                let explain = Some(self.explain_op(db, &p, *v).await?);
                Ok(Inner {
                    results: vec![],
                    write: None,
                    explain,
                    truncated: None,
                })
            }
            Op::CreateIndex(spec) => {
                // The statement was validated against the limit as written; check what exists too.
                let listed = db
                    .run_command(doc! { "listIndexes": &stmt.collection })
                    .await;
                let existing = listed
                    .ok()
                    .and_then(|r| {
                        r.get_document("cursor")
                            .ok()
                            .and_then(|c| c.get_array("firstBatch").ok().map(|a| a.len()))
                    })
                    .unwrap_or(1);
                if existing.saturating_sub(1) >= self.limits.max_indexes_per_collection {
                    return Err(PlaygroundError::Invalid(format!(
                        "`{}` already has {} indexes, the limit is {} per collection",
                        stmt.collection,
                        existing - 1,
                        self.limits.max_indexes_per_collection
                    )));
                }
                db.run_command(doc! {
                    "createIndexes": &stmt.collection,
                    "indexes": [ spec.clone() ],
                    "maxTimeMS": max_time,
                })
                .await?;
                Ok(empty(vec![Bson::Document(
                    doc! { "createdIndex": spec.get_str("name").unwrap_or("") },
                )]))
            }
            Op::DropIndex(name) => {
                db.run_command(doc! { "dropIndexes": &stmt.collection, "index": name })
                    .await?;
                Ok(empty(vec![Bson::Document(doc! { "droppedIndex": name })]))
            }
            Op::CreateSearchIndex(spec) => {
                create_search_index(db, &stmt.collection, spec).await?;
                Ok(empty(vec![Bson::Document(doc! {
                    "createdSearchIndex": spec.get_str("name").unwrap_or(""),
                    "type": spec.get_str("type").unwrap_or("search"),
                })]))
            }
        }
    }

    fn script_output(
        &self,
        outcome: std::result::Result<Result<()>, tokio::time::error::Elapsed>,
        mut outputs: Vec<StatementOutput>,
        budget_ms: u64,
        start: Instant,
    ) -> Result<ScriptOutput> {
        let error = match outcome {
            Err(_) => {
                let e = ErrorBody::from(&PlaygroundError::Timeout(budget_ms));
                if let Some(last) = outputs.last_mut().filter(|o| o.error.is_none()) {
                    // The statement that was running when the time ran out.
                    last.error = Some(ErrorBody {
                        code: e.code.clone(),
                        message: e.message.clone(),
                        hint: e.hint.clone(),
                    });
                }
                Some(e)
            }
            // Seeding failed (for example a duplicate _id in the data panel): nothing ran.
            Ok(Err(e)) if outputs.is_empty() => return Err(e),
            Ok(Err(e)) => Some(ErrorBody::from(&e)),
            Ok(Ok(())) => outputs.last().and_then(|o| {
                o.error.as_ref().map(|e| ErrorBody {
                    code: e.code.clone(),
                    message: e.message.clone(),
                    hint: e.hint.clone(),
                })
            }),
        };
        Ok(ScriptOutput {
            ok: error.is_none(),
            error,
            statements: outputs,
            elapsed_ms: start.elapsed().as_millis() as u64,
        })
    }
}

fn statement_prepared(collection: &str, query: Query) -> Prepared {
    Prepared {
        collections: Default::default(),
        collection: collection.to_string(),
        query,
        explain: None,
        indexes: Default::default(),
        search_indexes: Default::default(),
        sample_db: None,
    }
}

fn statement_output(stmt: &Statement, result: Result<Inner>, start: Instant) -> StatementOutput {
    let mut out = StatementOutput {
        index: stmt.index,
        line: stmt.line,
        statement: stmt.text.clone(),
        collection: stmt.collection.clone(),
        op: stmt.op.label(),
        results: vec![],
        write: None,
        explain: None,
        truncated: None,
        error: None,
        elapsed_ms: start.elapsed().as_millis() as u64,
    };
    match result {
        Ok(inner) => {
            out.results = inner
                .results
                .into_iter()
                .map(Bson::into_relaxed_extjson)
                .collect();
            out.write = inner.write;
            out.explain = inner.explain;
            out.truncated = inner.truncated;
        }
        Err(e) => out.error = Some(ErrorBody::from(&e)),
    }
    out
}
