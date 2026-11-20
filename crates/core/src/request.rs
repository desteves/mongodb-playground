use std::collections::BTreeMap;

use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::{json, Map, Value};

use crate::relaxed;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Find,
    Aggregate,
    Count,
    Distinct,
    Insert,
    Update,
    Delete,
    #[serde(rename = "listIndexes")]
    ListIndexes,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Find => "find",
            Mode::Aggregate => "aggregate",
            Mode::Count => "count",
            Mode::Distinct => "distinct",
            Mode::Insert => "insert",
            Mode::Update => "update",
            Mode::Delete => "delete",
            Mode::ListIndexes => "listIndexes",
        }
    }
}

/// How much detail MongoDB's `explain` should return.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Verbosity {
    QueryPlanner,
    ExecutionStats,
    AllPlansExecution,
}

impl Verbosity {
    pub fn as_str(self) -> &'static str {
        match self {
            Verbosity::QueryPlanner => "queryPlanner",
            Verbosity::ExecutionStats => "executionStats",
            Verbosity::AllPlansExecution => "allPlansExecution",
        }
    }
}

/// A run request as sent by the UI. Documents and the query are Extended JSON.
///
/// `query` shapes by mode:
/// - find:      `{ "filter": {..}, "projection": {..}?, "sort": {..}?, "limit": n? }`
/// - aggregate: `[ {stage}, ... ]`
/// - count:     `{ "filter": {..}? }`
/// - distinct:  `{ "key": "field", "filter": {..}? }`
/// - insert:    `{ "documents": [ {..}, ... ] }` (or `{ "document": {..} }`)
/// - update:    `{ "filter": {..}, "update": {..} | [..], "multi": bool?, "upsert": bool? }`
///   (an `update` without operators replaces the matched document)
/// - delete:    `{ "filter": {..}, "multi": bool? }`
/// - listIndexes: none (`{}` or null)
///
/// `collections`, `query` and `indexes` may each be a JSON value **or a string of relaxed JSON**
/// (comments and `/regex/flags` literals allowed), which is what the web editors send.
#[derive(Debug, Clone, Deserialize)]
pub struct RunRequest {
    pub mode: Mode,
    /// Collection name -> documents. Seeded into a throwaway database.
    #[serde(default, deserialize_with = "de_collections")]
    pub collections: BTreeMap<String, Vec<Value>>,
    /// The collection the query runs against.
    pub collection: String,
    #[serde(deserialize_with = "de_query")]
    pub query: Value,
    /// Indexes created after seeding and before the query, so explain can show `IXSCAN`.
    /// Collection name -> index specs. At most 3 per collection (`_id` does not count).
    #[serde(default, deserialize_with = "de_indexes")]
    pub indexes: BTreeMap<String, Vec<IndexSpec>>,
    /// One Atlas Search index per collection, built before the query (needs Atlas or `atlas local`).
    #[serde(
        default,
        rename = "searchIndexes",
        deserialize_with = "de_search_indexes"
    )]
    pub search_indexes: BTreeMap<String, SearchIndexSpec>,
    /// Query Atlas's sample datasets instead of your own documents (read-only).
    #[serde(default)]
    pub datasource: Option<Datasource>,
    /// Also return the query plan (insert cannot be explained).
    #[serde(default)]
    pub explain: Option<Verbosity>,
}

/// One index to create: `{"key": {"age": 1}, "options": {"unique": true}}`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndexSpec {
    pub key: Map<String, Value>,
    #[serde(default)]
    pub options: Map<String, Value>,
}

fn de_collections<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, Vec<Value>>, D::Error> {
    let setup = de_setup(d)?;
    let mut out = BTreeMap::new();
    for (name, c) in setup.collections {
        if !c.indexes.is_empty() || !c.search.is_empty() || !c.vector.is_empty() {
            return Err(de::Error::custom(format!(
                "collections: `{name}` declares indexes; per-collection indexes need the `script` request form (or use the separate `indexes` / `searchIndexes` fields)"
            )));
        }
        out.insert(name, c.documents);
    }
    Ok(out)
}

fn de_query<'de, D: Deserializer<'de>>(d: D) -> Result<Value, D::Error> {
    relaxed::value_or_text(Value::deserialize(d)?, "query").map_err(de::Error::custom)
}

fn de_indexes<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, Vec<IndexSpec>>, D::Error> {
    let v = relaxed::value_or_text(Value::deserialize(d)?, "indexes").map_err(de::Error::custom)?;
    if v.is_null() {
        return Ok(BTreeMap::new());
    }
    serde_json::from_value(v).map_err(|e| de::Error::custom(format!("indexes: {e}")))
}

/// `{"sample": "sample_mflix"}`: run against an Atlas sample database (find, aggregate, count,
/// distinct only; results are capped because the collections are large).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Datasource {
    Sample(String),
}

/// `{"name": "default", "definition": {"mappings": {"dynamic": true}}}`
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchIndexSpec {
    #[serde(default)]
    pub name: Option<String>,
    pub definition: Map<String, Value>,
}

fn de_search_indexes<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, SearchIndexSpec>, D::Error> {
    let v = relaxed::value_or_text(Value::deserialize(d)?, "searchIndexes")
        .map_err(de::Error::custom)?;
    if v.is_null() {
        return Ok(BTreeMap::new());
    }
    serde_json::from_value(v).map_err(|e| de::Error::custom(format!("searchIndexes: {e}")))
}

// ----- the unified "data panel": documents, indexes and search/vector indexes per collection -----

/// One collection as set up in the data panel.
#[derive(Debug, Clone, Default)]
pub struct CollectionSetup {
    pub documents: Vec<Value>,
    pub indexes: Vec<IndexSpec>,
    pub search: Vec<SearchIndexSpec>,
    pub vector: Vec<SearchIndexSpec>,
}

#[derive(Debug, Clone, Default)]
pub struct Setup {
    pub collections: BTreeMap<String, CollectionSetup>,
}

const SETUP_KEYS: &[&str] = &[
    "documents",
    "indexes",
    "searchIndexes",
    "searchIndex",
    "vectorSearchIndexes",
    "vectorSearchIndex",
];

fn one_or_many<T: serde::de::DeserializeOwned>(v: Value, what: &str) -> Result<Vec<T>, String> {
    let items = match v {
        Value::Array(a) => a,
        other => vec![other],
    };
    items
        .into_iter()
        .enumerate()
        .map(|(i, item)| serde_json::from_value(item).map_err(|e| format!("{what}[{i}]: {e}")))
        .collect()
}

/// Turn the data panel's value into a [`Setup`]. Per collection it takes either an array of
/// documents, or an object with `documents`, `indexes`, `searchIndexes` and `vectorSearchIndexes`.
pub fn setup_from_value(v: Value) -> Result<Setup, String> {
    let v = match v {
        Value::Null => return Ok(Setup::default()),
        // `{documents: [...], indexes: [...]}` at the top level means one collection.
        Value::Object(m)
            if m.contains_key("documents")
                && m.keys().all(|k| SETUP_KEYS.contains(&k.as_str())) =>
        {
            json!({ "collection": Value::Object(m) })
        }
        other => other,
    };
    let Value::Object(map) = v else {
        return Err(
            "the data panel must be an array of documents or an object of collections".into(),
        );
    };
    let mut setup = Setup::default();
    for (name, val) in map {
        let mut c = CollectionSetup::default();
        match val {
            Value::Array(docs) => c.documents = docs,
            Value::Object(m) => {
                for (key, v) in m {
                    let at = format!("{name}.{key}");
                    match key.as_str() {
                        "documents" => match v {
                            Value::Array(docs) => c.documents = docs,
                            _ => return Err(format!("{at} must be an array of documents")),
                        },
                        "indexes" => c.indexes = one_or_many(v, &at)?,
                        "searchIndexes" | "searchIndex" => c.search = one_or_many(v, &at)?,
                        "vectorSearchIndexes" | "vectorSearchIndex" => {
                            c.vector = one_or_many(v, &at)?
                        }
                        other => {
                            return Err(format!(
                                "{name}: unknown key `{other}`; use {}",
                                SETUP_KEYS.join(", ")
                            ))
                        }
                    }
                }
            }
            _ => return Err(format!("{name} must be an array of documents or an object")),
        }
        setup.collections.insert(name, c);
    }
    Ok(setup)
}

fn de_setup<'de, D: Deserializer<'de>>(d: D) -> Result<Setup, D::Error> {
    let v = relaxed::config_value(Value::deserialize(d)?, "data").map_err(de::Error::custom)?;
    setup_from_value(v).map_err(|e| de::Error::custom(format!("data: {e}")))
}

/// A shell-style request: seed the data panel, then run the script's statements in order.
#[derive(Debug, Clone, Deserialize)]
pub struct ScriptRequest {
    /// `db.people.find({...})`, `db.people.insertMany([...])`, ... several statements allowed.
    pub script: String,
    /// The data panel (`collections` is accepted as an alias).
    #[serde(default, alias = "collections", deserialize_with = "de_setup")]
    pub config: Setup,
    /// Explain the script's statements instead of running them (inserts and index builds still
    /// run, so the plans have data and indexes to work with).
    #[serde(default)]
    pub explain: Option<Verbosity>,
    #[serde(default)]
    pub datasource: Option<Datasource>,
}

/// Either a shell-style script request or the older structured single-operation request.
#[derive(Debug, Clone)]
pub enum RunBody {
    Script(ScriptRequest),
    Structured(RunRequest),
}

impl<'de> Deserialize<'de> for RunBody {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Value::deserialize(d)?;
        if v.get("script").is_some() {
            serde_json::from_value(v)
                .map(RunBody::Script)
                .map_err(de::Error::custom)
        } else {
            serde_json::from_value(v)
                .map(RunBody::Structured)
                .map_err(de::Error::custom)
        }
    }
}

/// `POST /api/v1/export`: take the data panel and script out of the site.
#[derive(Debug, Clone, Deserialize)]
pub struct ExportRequest {
    #[serde(default)]
    pub script: String,
    #[serde(default, alias = "collections", deserialize_with = "de_setup")]
    pub config: Setup,
    /// `mongosh` (a script that recreates everything; the default) or `json`.
    #[serde(default)]
    pub format: Option<String>,
}
