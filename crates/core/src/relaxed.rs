//! "Relaxed" input: text typed the way the MongoDB shell accepts it (see `shellvalue`),
//! converted to strict Extended JSON values. A request field may be a JSON value or a string
//! of such text, which is what the web editors send.

use serde_json::{json, Value};

use crate::{error::PlaygroundError, shellvalue};

type Result<T> = std::result::Result<T, PlaygroundError>;

/// Parse shell-style text (unquoted keys, `/regex/i`, `ObjectId(..)`, comments) into a value.
pub fn parse(text: &str, what: &str) -> Result<Value> {
    shellvalue::parse_value(text, what)
}

/// A request field may be a JSON value or a string of shell-style text.
pub fn value_or_text(v: Value, what: &str) -> Result<Value> {
    match v {
        Value::String(text) => parse(&text, what),
        other => Ok(other),
    }
}

/// The documents editor. Accepts `{coll: [docs], ...}`, `db = {...}`, or just `[docs]`, which
/// is a single collection named `collection` (the "single collection" template).
pub fn config_value(v: Value, what: &str) -> Result<Value> {
    let v = match v {
        Value::String(text) => shellvalue::parse_config(&text, what)?,
        other => other,
    };
    Ok(match v {
        Value::Array(docs) => json!({ "collection": docs }),
        other => other,
    })
}
