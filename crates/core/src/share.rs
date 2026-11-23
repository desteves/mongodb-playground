//! Share links: a playground saved under a random UUID id.
//!
//! The playground is stored as an opaque JSON string, exactly as sent (relaxed JSON text with
//! comments and `/regex/` literals included), so `$`-prefixed query keys never become BSON field
//! names. Identical content always gets the identical id. Shares expire 90 days after their last
//! view (a TTL index on `lastViewedAt`, refreshed on view at most once a day).

use mongodb::{
    bson::{doc, Bson, DateTime},
    Client,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    error::PlaygroundError,
    limits::Limits,
    request::{Mode, RunBody, Verbosity},
    shell::prepare_script,
    validate::prepare,
};

type Result<T> = std::result::Result<T, PlaygroundError>;

pub const SHARE_TTL_SECS: i64 = 90 * 24 * 60 * 60;
const REFRESH_AFTER_MS: i64 = 24 * 60 * 60 * 1000;
const BASE62: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// What a share link holds: the request fields, as typed (strings stay strings).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SharedPlayground {
    /// The shell-style script (the usual case). Structured playgrounds use mode/collection/query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collection: Option<String>,
    /// The data panel (`config` is accepted as an alias).
    #[serde(default, alias = "config", skip_serializing_if = "Value::is_null")]
    pub collections: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub query: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub indexes: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub search_indexes: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub datasource: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explain: Option<Verbosity>,
}

impl SharedPlayground {
    /// Only valid playgrounds can be shared (same rules as a run).
    pub fn validate(&self, limits: &Limits) -> Result<()> {
        let value =
            serde_json::to_value(self).map_err(|e| PlaygroundError::Invalid(e.to_string()))?;
        let body: RunBody =
            serde_json::from_value(value).map_err(|e| PlaygroundError::Invalid(e.to_string()))?;
        match body {
            RunBody::Script(req) => prepare_script(&req, limits).map(|_| ()),
            RunBody::Structured(req) => prepare(&req, limits).map(|_| ()),
        }
    }

    /// A hash of the content (not the link): identical playgrounds have the same content id.
    /// Stored with each share so duplicates can be found; links themselves are random UUIDs.
    pub fn content_id(&self) -> String {
        let bytes = serde_json::to_vec(self).unwrap_or_default();
        let digest = Sha256::digest(bytes);
        let mut n = u64::from_be_bytes(digest[..8].try_into().expect("8 bytes"));
        let mut out = Vec::new();
        while n > 0 {
            out.push(BASE62[(n % 62) as usize]);
            n /= 62;
        }
        out.reverse();
        String::from_utf8(out).unwrap_or_default()
    }
}

/// Share links are UUIDs (`8f14e45f-ceea-467f-a8d4-5c1f0b7e2a11`). Older short ids still load.
pub fn is_valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

pub struct Shares {
    client: Client,
    db: String,
    /// Most shares kept in total; saving stops (503) when the collection is this full.
    max_total: u64,
    /// Most shares one client address may create per UTC day (counted in memory, so per server instance).
    per_client_per_day: u32,
    quota: std::sync::Mutex<std::collections::HashMap<String, (u64, u32)>>,
}

/// How much the shares collection holds.
#[derive(Debug, Clone, Copy)]
pub struct Usage {
    pub count: u64,
    pub bytes: u64,
}

#[derive(Debug)]
pub struct Loaded {
    pub playground: SharedPlayground,
    pub created_at_ms: i64,
}

impl Shares {
    pub fn new(client: Client, db: impl Into<String>) -> Self {
        Self {
            client,
            db: db.into(),
            max_total: 100_000,
            per_client_per_day: 30,
            quota: Default::default(),
        }
    }

    /// Cap the total number of shares and the number one client may create per day.
    pub fn with_limits(mut self, max_total: u64, per_client_per_day: u32) -> Self {
        self.max_total = max_total;
        self.per_client_per_day = per_client_per_day;
        self
    }

    pub fn max_total(&self) -> u64 {
        self.max_total
    }

    /// Count against `client`'s daily allowance; `Err(seconds)` until the next UTC day when it is used up.
    fn take_quota(&self, client: &str, now_secs: u64) -> std::result::Result<(), u64> {
        let day = now_secs / 86_400;
        let mut q = self.quota.lock().unwrap_or_else(|e| e.into_inner());
        if q.len() > 50_000 {
            q.retain(|_, (d, _)| *d == day);
        }
        let entry = q.entry(client.to_string()).or_insert((day, 0));
        if entry.0 != day {
            *entry = (day, 0);
        }
        if entry.1 >= self.per_client_per_day {
            return Err((day + 1) * 86_400 - now_secs);
        }
        entry.1 += 1;
        Ok(())
    }

    /// Number of documents and their size, from collection statistics (cheap).
    pub async fn usage(&self) -> Result<Usage> {
        let r = self
            .client
            .database(&self.db)
            .run_command(doc! { "collStats": "shares" })
            .await;
        // A collection that does not exist yet is empty.
        let Ok(r) = r else {
            return Ok(Usage { count: 0, bytes: 0 });
        };
        let num = |k: &str| {
            r.get(k)
                .and_then(|v| match v {
                    Bson::Int32(n) => Some(*n as u64),
                    Bson::Int64(n) => Some(*n as u64),
                    Bson::Double(n) => Some(*n as u64),
                    _ => None,
                })
                .unwrap_or(0)
        };
        Ok(Usage {
            count: num("count"),
            bytes: num("size"),
        })
    }

    /// Delete one share (a takedown). True when it existed.
    pub async fn remove(&self, id: &str) -> Result<bool> {
        if !is_valid_id(id) {
            return Ok(false);
        }
        let r = self
            .client
            .database(&self.db)
            .run_command(doc! {
                "delete": "shares",
                "deletes": [ { "q": { "_id": id }, "limit": 1 } ],
            })
            .await?;
        Ok(r.get_i32("n").unwrap_or(0) > 0)
    }

    /// `save`, for a known client: enforces the per-client daily quota and the total cap first.
    pub async fn save_for(
        &self,
        client: &str,
        p: &SharedPlayground,
        limits: &Limits,
    ) -> Result<String> {
        p.validate(limits)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if let Err(wait) = self.take_quota(client, now) {
            return Err(PlaygroundError::RateLimited(wait));
        }
        if self.usage().await?.count >= self.max_total {
            return Err(PlaygroundError::Full(
                "share storage is full right now; no new share links can be created".into(),
            ));
        }
        self.save(p, limits).await
    }

    /// TTL index so unviewed shares expire. Safe to call on every start.
    pub async fn ensure_indexes(&self) -> Result<()> {
        self.client
            .database(&self.db)
            .run_command(doc! {
                "createIndexes": "shares",
                "indexes": [ {
                    "key": { "lastViewedAt": 1 },
                    "name": "ttl_lastViewedAt",
                    "expireAfterSeconds": SHARE_TTL_SECS,
                } ],
            })
            .await?;
        Ok(())
    }

    pub async fn save(&self, p: &SharedPlayground, limits: &Limits) -> Result<String> {
        p.validate(limits)?;
        let payload =
            serde_json::to_string(p).map_err(|e| PlaygroundError::Invalid(e.to_string()))?;
        if payload.len() > limits.max_share_bytes {
            return Err(PlaygroundError::Invalid(format!(
                "this playground is {} bytes; the limit for share links is {}",
                payload.len(),
                limits.max_share_bytes
            )));
        }
        // Every share is its own link: a random UUID, so a link cannot be guessed from the content
        // and sharing twice gives two independent links (each with its own 90-day clock).
        let id = uuid::Uuid::new_v4().to_string();
        let now = DateTime::now();
        let res = self
            .client
            .database(&self.db)
            .run_command(doc! {
                "insert": "shares",
                "documents": [ {
                    "_id": &id,
                    "payload": payload,
                    "contentId": p.content_id(),
                    "createdAt": now,
                    "lastViewedAt": now,
                    "v": 2,
                } ],
            })
            .await?;
        if let Ok(errs) = res.get_array("writeErrors") {
            if !errs.is_empty() {
                return Err(PlaygroundError::Database(
                    "could not save the share link".into(),
                ));
            }
        }
        Ok(id)
    }

    pub async fn load(&self, id: &str) -> Result<Loaded> {
        let not_found =
            || PlaygroundError::NotFound(format!("no shared playground with id `{id}`"));
        if !is_valid_id(id) {
            return Err(not_found());
        }
        let db = self.client.database(&self.db);
        let res = db
            .run_command(
                doc! { "find": "shares", "filter": { "_id": id }, "limit": 1, "singleBatch": true },
            )
            .await?;
        let found = res
            .get_document("cursor")
            .ok()
            .and_then(|c| c.get_array("firstBatch").ok())
            .and_then(|b| b.first())
            .and_then(|d| {
                if let Bson::Document(d) = d {
                    Some(d.clone())
                } else {
                    None
                }
            })
            .ok_or_else(not_found)?;

        let payload = found.get_str("payload").map_err(|_| not_found())?;
        let playground: SharedPlayground = serde_json::from_str(payload)
            .map_err(|_| PlaygroundError::Database("stored playground is unreadable".into()))?;
        let created_at_ms = found
            .get_datetime("createdAt")
            .map(|d| d.timestamp_millis())
            .unwrap_or(0);

        // Keep a viewed share alive: refresh the TTL clock, at most once a day.
        let last = found
            .get_datetime("lastViewedAt")
            .map(|d| d.timestamp_millis())
            .unwrap_or(0);
        if DateTime::now().timestamp_millis() - last > REFRESH_AFTER_MS {
            let _ = db
                .run_command(doc! {
                    "update": "shares",
                    "updates": [ { "q": { "_id": id }, "u": { "$set": { "lastViewedAt": DateTime::now() } } } ],
                })
                .await;
        }
        Ok(Loaded {
            playground,
            created_at_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn sample() -> SharedPlayground {
        serde_json::from_value(json!({
            "mode": "find",
            "collection": "people",
            "collections": "{\"people\": [{\"name\": \"Ada\"}]}",
            "query": "{\"filter\": {\"name\": /^a/i}}",
        }))
        .unwrap()
    }

    #[test]
    fn content_ids_are_deterministic_and_links_are_uuids() {
        let a = sample();
        let mut b = sample();
        assert_eq!(a.content_id(), b.content_id());
        b.collection = Some("other".into());
        assert_ne!(a.content_id(), b.content_id());
        let link = uuid::Uuid::new_v4().to_string();
        assert!(is_valid_id(&link), "{link}");
        assert_eq!(link.len(), 36);
    }

    #[test]
    fn valid_playgrounds_validate_and_invalid_ones_do_not() {
        assert!(sample().validate(&Limits::default()).is_ok());
        let mut bad = sample();
        bad.query = json!({"filter": {"$where": "1"}});
        assert!(matches!(
            bad.validate(&Limits::default()),
            Err(PlaygroundError::Forbidden(_))
        ));
    }

    #[tokio::test]
    async fn the_daily_quota_counts_per_client_and_resets_each_day() {
        let client = Client::with_uri_str("mongodb://localhost:1/?serverSelectionTimeoutMS=100")
            .await
            .unwrap();
        let s = Shares::new(client, "x").with_limits(10, 2);
        let day = 20_000 * 86_400;
        assert!(s.take_quota("a", day + 10).is_ok());
        assert!(s.take_quota("a", day + 20).is_ok());
        assert_eq!(s.take_quota("a", day + 86_000), Err(400));
        assert!(
            s.take_quota("b", day + 30).is_ok(),
            "another client has its own allowance"
        );
        assert!(
            s.take_quota("a", day + 86_400).is_ok(),
            "a new day starts over"
        );
        assert_eq!(s.max_total(), 10);
    }

    #[test]
    fn id_validation() {
        assert!(is_valid_id("aZ09"));
        assert!(is_valid_id("8f14e45f-ceea-467f-a8d4-5c1f0b7e2a11"));
        assert!(!is_valid_id("../etc/passwd"));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("a/b"));
        assert!(!is_valid_id("a b"));
        assert!(!is_valid_id(&"x".repeat(41)));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let r = serde_json::from_value::<SharedPlayground>(json!({
            "mode": "find", "collection": "c", "query": {}, "surprise": 1
        }));
        assert!(r.is_err());
    }
}
