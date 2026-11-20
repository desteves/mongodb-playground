//! Usage stats for the `playground-platform` database: one small event per interaction.
//!
//! Privacy rule: **no query text, no documents, no raw IP addresses.** Only metadata: what kind
//! of action, which mode, how long, whether it worked, and salted daily-rotating hashes.
//! Writes are fire-and-forget through a bounded queue; if it is full the event is dropped and
//! the user's request is never slowed down.

use std::time::Duration;

use mongodb::{
    bson::{doc, Bson, DateTime, Document},
    Client,
};
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};

const QUEUE: usize = 2048;
const BATCH: usize = 100;
const FLUSH_EVERY: Duration = Duration::from_secs(2);
const RETENTION_SECS: i64 = 13 * 30 * 24 * 60 * 60;

/// Event types the UI may report through `POST /api/v1/events` (server-side ones are recorded directly).
pub const UI_EVENT_TYPES: &[&str] = &[
    "page_view",
    "format",
    "mode_change",
    "example_open",
    "datasource_change",
    "tab_change",
    "copy_link",
];

#[derive(Debug, Clone, Default)]
pub struct Event {
    /// `run`, `explain`, `validate`, `share`, `load_share`, `mcp_call`, `error`, or a UI type.
    pub kind: String,
    /// `web`, `desktop` or `agent`.
    pub client: String,
    pub mode: Option<String>,
    pub datasource: Option<String>,
    pub dur_ms: Option<i64>,
    pub ok: bool,
    pub err_code: Option<String>,
    pub bytes_in: Option<i64>,
    pub rows: Option<i64>,
    pub share_id: Option<String>,
    pub example_id: Option<String>,
    pub session_id: Option<String>,
    pub ip_hash: Option<String>,
    pub country: Option<String>,
}

impl Event {
    /// The stored document. `env` goes in `meta` with the other low-cardinality fields.
    pub fn to_document(&self, env: &str) -> Document {
        let mut meta = doc! { "env": env, "client": &self.client };
        if let Some(m) = &self.mode {
            meta.insert("mode", m);
        }
        if let Some(d) = &self.datasource {
            meta.insert("datasource", d);
        }
        let mut d = doc! { "ts": DateTime::now(), "meta": meta, "type": &self.kind, "ok": self.ok };
        let mut put = |k: &str, v: Option<Bson>| {
            if let Some(v) = v {
                d.insert(k, v);
            }
        };
        put("durMs", self.dur_ms.map(Bson::Int64));
        put("errCode", self.err_code.clone().map(Bson::String));
        put("bytesIn", self.bytes_in.map(Bson::Int64));
        put("rows", self.rows.map(Bson::Int64));
        put("shareId", self.share_id.clone().map(Bson::String));
        put("exampleId", self.example_id.clone().map(Bson::String));
        put("sessionId", self.session_id.clone().map(Bson::String));
        put("ipHash", self.ip_hash.clone().map(Bson::String));
        put("country", self.country.clone().map(Bson::String));
        d
    }
}

/// Salted hash of an address, rotating daily (`day` is days since the epoch), so activity can
/// be counted within a day without ever storing or linking raw addresses.
pub fn ip_hash(ip: &str, salt: &str, day: u64) -> String {
    let digest = Sha256::digest(format!("{salt}|{day}|{ip}"));
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// Session ids are random strings from the client; keep them short and boring.
pub fn clean_session_id(s: &str) -> Option<String> {
    (!s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    .then(|| s.to_string())
}

enum Msg {
    Event(Box<Event>),
    Flush(oneshot::Sender<()>),
}

#[derive(Clone)]
pub struct Stats {
    tx: Option<mpsc::Sender<Msg>>,
}

impl Stats {
    /// A handle that records nothing (tests, or stats turned off).
    pub fn disabled() -> Self {
        Self { tx: None }
    }

    /// Start the background writer. Creates the time-series collection if it is missing.
    pub fn start(client: Client, db: impl Into<String>, env: impl Into<String>) -> Self {
        let (tx, mut rx) = mpsc::channel::<Msg>(QUEUE);
        let (db, env) = (db.into(), env.into());
        tokio::spawn(async move {
            if let Err(e) = ensure_schema(&client, &db).await {
                tracing::warn!(error = %e, "stats: could not prepare the events collection");
            }
            let mut batch: Vec<Document> = Vec::new();
            let mut tick = tokio::time::interval(FLUSH_EVERY);
            loop {
                tokio::select! {
                    msg = rx.recv() => match msg {
                        Some(Msg::Event(e)) => {
                            batch.push(e.to_document(&env));
                            if batch.len() >= BATCH {
                                write(&client, &db, &mut batch).await;
                            }
                        }
                        Some(Msg::Flush(done)) => {
                            write(&client, &db, &mut batch).await;
                            let _ = done.send(());
                        }
                        None => {
                            write(&client, &db, &mut batch).await;
                            break;
                        }
                    },
                    _ = tick.tick() => write(&client, &db, &mut batch).await,
                }
            }
        });
        Self { tx: Some(tx) }
    }

    /// Never blocks and never fails the caller: a full queue drops the event.
    pub fn record(&self, event: Event) {
        if let Some(tx) = &self.tx {
            let _ = tx.try_send(Msg::Event(Box::new(event)));
        }
    }

    /// Write everything queued so far (used by tests and at shutdown).
    pub async fn flush(&self) {
        if let Some(tx) = &self.tx {
            let (done, wait) = oneshot::channel();
            if tx.send(Msg::Flush(done)).await.is_ok() {
                let _ = wait.await;
            }
        }
    }
}

async fn ensure_schema(client: &Client, db: &str) -> Result<(), mongodb::error::Error> {
    let res = client
        .database(db)
        .run_command(doc! {
            "create": "events",
            "timeseries": { "timeField": "ts", "metaField": "meta", "granularity": "seconds" },
            "expireAfterSeconds": RETENTION_SECS,
        })
        .await;
    match res {
        Ok(_) => Ok(()),
        // 48 = NamespaceExists: already created.
        Err(e) if matches!(&*e.kind, mongodb::error::ErrorKind::Command(c) if c.code == 48) => {
            Ok(())
        }
        Err(e) => Err(e),
    }
}

async fn write(client: &Client, db: &str, batch: &mut Vec<Document>) {
    if batch.is_empty() {
        return;
    }
    let docs: Vec<Bson> = batch.drain(..).map(Bson::Document).collect();
    let n = docs.len();
    if let Err(e) = client
        .database(db)
        .run_command(doc! { "insert": "events", "documents": docs })
        .await
    {
        tracing::warn!(error = %e, dropped = n, "stats: could not write events");
    }
}

/// Fold recent raw events into `daily_rollups` (counts, errors, p50/p95 latency per day, type,
/// mode, datasource and client). Idempotent: re-running replaces the same rows.
pub async fn rollup(
    client: &Client,
    db: &str,
    last_hours: i64,
) -> Result<(), mongodb::error::Error> {
    let since = DateTime::from_millis(DateTime::now().timestamp_millis() - last_hours * 3_600_000);
    client
        .database(db)
        .run_command(doc! {
            "aggregate": "events",
            "pipeline": [
                { "$match": { "ts": { "$gte": since } } },
                { "$group": {
                    "_id": {
                        "day": { "$dateToString": { "format": "%Y-%m-%d", "date": "$ts" } },
                        "type": "$type",
                        "mode": "$meta.mode",
                        "datasource": "$meta.datasource",
                        "client": "$meta.client",
                    },
                    "count": { "$sum": 1 },
                    "errors": { "$sum": { "$cond": ["$ok", 0, 1] } },
                    "latencyMs": { "$percentile": { "input": "$durMs", "p": [0.5, 0.95], "method": "approximate" } },
                } },
                { "$merge": { "into": "daily_rollups", "on": "_id", "whenMatched": "replace", "whenNotMatched": "insert" } },
            ],
            "cursor": {},
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_carry_metadata_only() {
        let e = Event {
            kind: "run".into(),
            client: "agent".into(),
            mode: Some("find".into()),
            dur_ms: Some(12),
            ok: true,
            rows: Some(3),
            ..Default::default()
        };
        let d = e.to_document("prod");
        assert_eq!(d.get_str("type").unwrap(), "run");
        assert_eq!(
            d.get_document("meta").unwrap().get_str("client").unwrap(),
            "agent"
        );
        assert_eq!(
            d.get_document("meta").unwrap().get_str("env").unwrap(),
            "prod"
        );
        assert_eq!(d.get_i64("rows").unwrap(), 3);
        assert!(d.get("durMs").is_some() && d.get("shareId").is_none());
        // Only these keys may ever exist: no query, documents or address fields.
        let allowed = [
            "ts",
            "meta",
            "type",
            "ok",
            "durMs",
            "errCode",
            "bytesIn",
            "rows",
            "shareId",
            "exampleId",
            "sessionId",
            "ipHash",
            "country",
        ];
        assert!(d.keys().all(|k| allowed.contains(&k.as_str())));
    }

    #[test]
    fn ip_hash_is_salted_daily_and_not_the_address() {
        let a = ip_hash("203.0.113.7", "salt", 20_000);
        assert_eq!(a, ip_hash("203.0.113.7", "salt", 20_000));
        assert_ne!(a, ip_hash("203.0.113.7", "salt", 20_001), "rotates daily");
        assert_ne!(
            a,
            ip_hash("203.0.113.7", "other", 20_000),
            "depends on the salt"
        );
        assert!(!a.contains("203"));
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn session_ids_are_sanitised() {
        assert_eq!(clean_session_id("abc-123_X").as_deref(), Some("abc-123_X"));
        assert!(clean_session_id("").is_none());
        assert!(clean_session_id("has space").is_none());
        assert!(clean_session_id(&"a".repeat(65)).is_none());
    }

    #[test]
    fn disabled_stats_accept_events() {
        Stats::disabled().record(Event::default());
    }
}
