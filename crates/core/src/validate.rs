use std::collections::BTreeMap;

use mongodb::bson::{doc, Bson, Document};
use serde_json::{Map, Value};

use crate::{
    error::PlaygroundError,
    limits::Limits,
    request::{Datasource, IndexSpec, Mode, RunRequest, SearchIndexSpec, Verbosity},
};

type Result<T> = std::result::Result<T, PlaygroundError>;

/// Operators and stages that are never allowed anywhere in a request.
///
/// Server-side JavaScript, writes outside the throwaway collection, and admin/diagnostic
/// stages. This is the primary defence: the runner's database user cannot be scoped to a
/// per-run database name, so these checks are what keeps a query inside its own sandbox.
const FORBIDDEN: &[&str] = &[
    "$where",
    "$function",
    "$accumulator",
    "$out",
    "$merge",
    "$currentOp",
    "$listSessions",
    "$listLocalSessions",
    "$listSampledQueries",
    "$listSearchIndexes",
    "$collStats",
    "$indexStats",
    "$planCacheStats",
    "$queryStats",
    "$changeStream",
    "$changeStreamSplitLargeEvent",
    "$shardedDataDistribution",
    "$documents",
];

#[derive(Debug, Clone)]
pub enum Query {
    Find {
        filter: Document,
        projection: Option<Document>,
        sort: Option<Document>,
        /// `None` means no limit.
        limit: Option<i64>,
        skip: Option<i64>,
    },
    Aggregate {
        pipeline: Vec<Document>,
    },
    Count {
        filter: Document,
    },
    Distinct {
        key: String,
        filter: Document,
    },
    Insert {
        documents: Vec<Document>,
    },
    Update {
        filter: Document,
        /// An operator/replacement document, or a pipeline array.
        update: Bson,
        multi: bool,
        upsert: bool,
    },
    Delete {
        filter: Document,
        multi: bool,
    },
    ListIndexes,
}

impl Query {
    pub fn mode(&self) -> Mode {
        match self {
            Query::Find { .. } => Mode::Find,
            Query::Aggregate { .. } => Mode::Aggregate,
            Query::Count { .. } => Mode::Count,
            Query::Distinct { .. } => Mode::Distinct,
            Query::Insert { .. } => Mode::Insert,
            Query::Update { .. } => Mode::Update,
            Query::Delete { .. } => Mode::Delete,
            Query::ListIndexes => Mode::ListIndexes,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Prepared {
    pub collections: BTreeMap<String, Vec<Document>>,
    pub collection: String,
    pub query: Query,
    pub explain: Option<Verbosity>,
    /// Ready-to-send `createIndexes` specs per collection (with generated names).
    pub indexes: BTreeMap<String, Vec<Document>>,
    /// Ready-to-send `createSearchIndexes` specs, one per collection.
    pub search_indexes: BTreeMap<String, Vec<Document>>,
    /// Set for sample-data runs: the (read-only) database to query instead of a throwaway one.
    pub sample_db: Option<String>,
}

/// Validate a request against `limits` and convert it to BSON.
pub fn prepare(req: &RunRequest, limits: &Limits) -> Result<Prepared> {
    check_payload_size(req, limits)?;

    if let Some(Datasource::Sample(db)) = &req.datasource {
        return prepare_sample(req, db, limits);
    }
    if req.collections.is_empty() {
        return Err(invalid("add at least one collection with documents"));
    }
    if req.collections.len() > limits.max_collections {
        return Err(invalid(format!(
            "at most {} collections are allowed",
            limits.max_collections
        )));
    }

    let mut collections = BTreeMap::new();
    for (name, docs) in &req.collections {
        check_collection_name(name)?;
        if docs.len() > limits.max_docs_per_collection {
            return Err(invalid(format!(
                "collection `{name}` has {} documents, the limit is {}",
                docs.len(),
                limits.max_docs_per_collection
            )));
        }
        let mut converted = Vec::with_capacity(docs.len());
        for (i, d) in docs.iter().enumerate() {
            let doc = to_document(d, &format!("{name}[{i}]"))?;
            walk(&Bson::Document(doc.clone()), 0, limits)?;
            converted.push(doc);
        }
        collections.insert(name.clone(), converted);
    }

    let Some(existing) = collections.get(&req.collection) else {
        return Err(invalid(format!(
            "collection `{}` is not defined in the config",
            req.collection
        )));
    };
    let existing = existing.len();

    let query = match req.mode {
        Mode::Find => parse_find(&req.query)?,
        Mode::Aggregate => parse_aggregate(&req.query)?,
        Mode::Count => parse_count(&req.query)?,
        Mode::Distinct => parse_distinct(&req.query)?,
        Mode::Insert => parse_insert(&req.query)?,
        Mode::Update => parse_update(&req.query)?,
        Mode::Delete => parse_delete(&req.query)?,
        Mode::ListIndexes => parse_list_indexes(&req.query)?,
    };
    walk_query(&query, limits)?;
    let indexes = prepare_indexes(&req.indexes, &collections, limits)?;
    let search_indexes = prepare_search_indexes(&req.search_indexes, &collections, limits)?;
    if query_uses_stage(&query, &["$vectorSearch"]) {
        return Err(invalid(
            "$vectorSearch needs a vector search index: use the `script` request form and declare vectorSearchIndexes in the data panel",
        ));
    }
    if query_uses_search(&query) && search_indexes.is_empty() {
        return Err(invalid(
            "$search and $searchMeta need a search index: add `searchIndexes` for the collection (one per collection)",
        ));
    }

    if req.explain.is_some() && matches!(query, Query::Insert { .. } | Query::ListIndexes) {
        return Err(invalid(format!(
            "{} cannot be explained; explain a find, aggregate, count, distinct, update or delete",
            req.mode.as_str()
        )));
    }
    if let Query::Insert { documents } = &query {
        if existing + documents.len() > limits.max_docs_per_collection {
            return Err(invalid(format!(
                "inserting {} documents into `{}` ({existing} already there) would exceed the limit of {} per collection",
                documents.len(),
                req.collection,
                limits.max_docs_per_collection
            )));
        }
    }

    Ok(Prepared {
        collections,
        collection: req.collection.clone(),
        query,
        explain: req.explain,
        indexes,
        search_indexes,
        sample_db: None,
    })
}

pub(crate) fn check_sample_name(db: &str) -> Result<()> {
    let ok = db.len() <= 64
        && db.strip_prefix("sample_").is_some_and(|rest| {
            !rest.is_empty()
                && rest
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        });
    if !ok {
        return Err(invalid(format!(
            "`{db}` is not a sample database; names look like sample_mflix"
        )));
    }
    Ok(())
}

/// Sample-data runs: read-only queries against an Atlas `sample_*` database.
fn prepare_sample(req: &RunRequest, db: &str, limits: &Limits) -> Result<Prepared> {
    check_sample_name(db)?;
    if !req.collections.is_empty() || !req.indexes.is_empty() || !req.search_indexes.is_empty() {
        return Err(invalid(
            "sample data is read-only: remove `collections`, `indexes` and `searchIndexes`, or drop `datasource`",
        ));
    }
    check_collection_name(&req.collection)?;
    let query = match req.mode {
        Mode::Find => parse_find(&req.query)?,
        Mode::Aggregate => parse_aggregate(&req.query)?,
        Mode::Count => parse_count(&req.query)?,
        Mode::Distinct => parse_distinct(&req.query)?,
        other => {
            return Err(invalid(format!(
            "sample data is read-only: {} is not available; use find, aggregate, count or distinct",
            other.as_str()
        )))
        }
    };
    walk_query(&query, limits)?;
    Ok(Prepared {
        collections: BTreeMap::new(),
        collection: req.collection.clone(),
        query,
        explain: req.explain,
        indexes: BTreeMap::new(),
        search_indexes: BTreeMap::new(),
        sample_db: Some(db.to_string()),
    })
}

/// Does an aggregation use any of these stages (anywhere, including nested pipelines)?
pub(crate) fn query_uses_stage(q: &Query, stages: &[&str]) -> bool {
    fn has(b: &Bson, stages: &[&str]) -> bool {
        match b {
            Bson::Document(d) => d
                .iter()
                .any(|(k, v)| stages.contains(&k.as_str()) || has(v, stages)),
            Bson::Array(a) => a.iter().any(|v| has(v, stages)),
            _ => false,
        }
    }
    match q {
        Query::Aggregate { pipeline } => pipeline
            .iter()
            .any(|s| has(&Bson::Document(s.clone()), stages)),
        _ => false,
    }
}

fn query_uses_search(q: &Query) -> bool {
    query_uses_stage(q, &["$search", "$searchMeta"])
}

/// Search index definition keys users may set (no `fields`: that is a vector index).
const SEARCH_DEFINITION_KEYS: &[&str] = &[
    "mappings",
    "analyzer",
    "analyzers",
    "searchAnalyzer",
    "synonyms",
    "storedSource",
];

fn prepare_search_indexes(
    requested: &BTreeMap<String, SearchIndexSpec>,
    collections: &BTreeMap<String, Vec<Document>>,
    limits: &Limits,
) -> Result<BTreeMap<String, Vec<Document>>> {
    let mut out = BTreeMap::new();
    for (coll, spec) in requested {
        if !collections.contains_key(coll) {
            return Err(invalid(format!(
                "searchIndexes.{coll}: collection `{coll}` is not defined in the config"
            )));
        }
        out.insert(
            coll.clone(),
            vec![prepare_search_index(coll, spec, limits, false)?],
        );
    }
    Ok(out)
}

/// Validate one Atlas Search (`vector == false`) or Vector Search index definition and build the
/// `createSearchIndexes` spec. Search: `mappings` required. Vector: `fields` of vector/filter type.
pub(crate) fn prepare_search_index(
    coll: &str,
    spec: &SearchIndexSpec,
    limits: &Limits,
    vector: bool,
) -> Result<Document> {
    let kind = if vector { "vectorSearch" } else { "search" };
    let at = format!("{kind} index on `{coll}`");
    let name = spec
        .name
        .clone()
        .unwrap_or_else(|| if vector { "vector_index" } else { "default" }.to_string());
    if name.is_empty()
        || name.len() > 64
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(invalid(format!(
            "{at}: `name` must be 1-64 letters, digits or `_`"
        )));
    }
    let allowed: &[&str] = if vector {
        &["fields"]
    } else {
        SEARCH_DEFINITION_KEYS
    };
    if let Some(bad) = spec
        .definition
        .keys()
        .find(|k| !allowed.contains(&k.as_str()))
    {
        return Err(invalid(format!(
            "{at}: definition key `{bad}` is not allowed; allowed: {}",
            allowed.join(", ")
        )));
    }
    if vector {
        check_vector_fields(&at, spec.definition.get("fields"))?;
    } else if !spec.definition.contains_key("mappings") {
        return Err(invalid(format!(
            "{at}: the definition needs `mappings`, for example {{\"mappings\": {{\"dynamic\": true}}}}"
        )));
    }
    let definition = to_document(
        &Value::Object(spec.definition.clone()),
        &format!("{at}.definition"),
    )?;
    walk(&Bson::Document(definition.clone()), 0, limits)?;
    Ok(doc! { "name": name, "type": kind, "definition": definition })
}

/// `fields: [{type: "vector", path, numDimensions, similarity}, {type: "filter", path}]`
fn check_vector_fields(at: &str, fields: Option<&Value>) -> Result<()> {
    let Some(Value::Array(fields)) = fields else {
        return Err(invalid(format!(
            "{at}: the definition needs `fields`, for example {{\"fields\": [{{\"type\": \"vector\", \"path\": \"embedding\", \"numDimensions\": 3, \"similarity\": \"cosine\"}}]}}"
        )));
    };
    let mut vectors = 0;
    for (i, f) in fields.iter().enumerate() {
        let obj = f
            .as_object()
            .ok_or_else(|| invalid(format!("{at}: fields[{i}] must be an object")))?;
        if !obj
            .get("path")
            .is_some_and(|p| p.as_str().is_some_and(|p| !p.is_empty()))
        {
            return Err(invalid(format!("{at}: fields[{i}] needs a `path`")));
        }
        match obj.get("type").and_then(Value::as_str) {
            Some("vector") => {
                vectors += 1;
                let dims = obj.get("numDimensions").and_then(Value::as_i64);
                if !dims.is_some_and(|d| (1..=4096).contains(&d)) {
                    return Err(invalid(format!(
                        "{at}: fields[{i}].numDimensions must be 1 to 4096"
                    )));
                }
                if !matches!(
                    obj.get("similarity").and_then(Value::as_str),
                    Some("euclidean" | "cosine" | "dotProduct")
                ) {
                    return Err(invalid(format!(
                        "{at}: fields[{i}].similarity must be euclidean, cosine or dotProduct"
                    )));
                }
            }
            Some("filter") => {}
            _ => {
                return Err(invalid(format!(
                    "{at}: fields[{i}].type must be \"vector\" or \"filter\""
                )))
            }
        }
    }
    if vectors == 0 {
        return Err(invalid(format!(
            "{at}: add at least one field of type \"vector\""
        )));
    }
    Ok(())
}

pub(crate) fn walk_query(q: &Query, limits: &Limits) -> Result<()> {
    let doc = |d: &Document| walk(&Bson::Document(d.clone()), 0, limits);
    match q {
        Query::Find {
            filter,
            projection,
            sort,
            ..
        } => {
            doc(filter)?;
            projection.as_ref().map(doc).transpose()?;
            sort.as_ref().map(doc).transpose()?;
        }
        Query::Aggregate { pipeline } => pipeline.iter().try_for_each(doc)?,
        Query::Count { filter } | Query::Distinct { filter, .. } | Query::Delete { filter, .. } => {
            doc(filter)?
        }
        Query::Insert { documents } => documents.iter().try_for_each(doc)?,
        Query::Update { filter, update, .. } => {
            doc(filter)?;
            walk(update, 0, limits)?;
        }
        Query::ListIndexes => {}
    }
    Ok(())
}

fn check_payload_size(req: &RunRequest, limits: &Limits) -> Result<()> {
    let size = serde_json::to_vec(&req.collections)
        .map(|v| v.len())
        .unwrap_or(usize::MAX)
        + serde_json::to_vec(&req.query)
            .map(|v| v.len())
            .unwrap_or(usize::MAX);
    if size > limits.max_payload_bytes {
        return Err(invalid(format!(
            "payload is {size} bytes, the limit is {}",
            limits.max_payload_bytes
        )));
    }
    Ok(())
}

/// Collection names: ASCII letters, digits, `_`; must start with a letter; at most 64 chars.
pub(crate) fn check_collection_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let ok = name.len() <= 64
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(invalid(format!(
            "invalid collection name `{name}`: use letters, digits and `_`, starting with a letter (max 64)"
        )))
    }
}

/// The query as an object whose keys are all in `allowed`.
fn object<'a>(q: &'a Value, allowed: &[&str], mode: &str) -> Result<&'a Map<String, Value>> {
    let obj = q.as_object().ok_or_else(|| {
        invalid(format!(
            "{mode} query must be an object with: {}",
            allowed.join(", ")
        ))
    })?;
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(invalid(format!(
                "unknown {mode} option `{key}`; allowed: {}",
                allowed.join(", ")
            )));
        }
    }
    Ok(obj)
}

fn opt_doc(obj: &Map<String, Value>, key: &str) -> Result<Option<Document>> {
    obj.get(key).map(|v| to_document(v, key)).transpose()
}

fn doc_or_empty(obj: &Map<String, Value>, key: &str) -> Result<Document> {
    Ok(opt_doc(obj, key)?.unwrap_or_default())
}

fn opt_bool(obj: &Map<String, Value>, key: &str) -> Result<bool> {
    match obj.get(key) {
        None => Ok(false),
        Some(v) => v
            .as_bool()
            .ok_or_else(|| invalid(format!("`{key}` must be true or false"))),
    }
}

fn parse_find(q: &Value) -> Result<Query> {
    let obj = object(
        q,
        &["filter", "projection", "sort", "limit", "skip"],
        "find",
    )?;
    let skip = match obj.get("skip") {
        None => None,
        Some(v) => match v.as_i64() {
            Some(n) if n >= 0 => Some(n).filter(|n| *n > 0),
            _ => return Err(invalid("`skip` must be a non-negative integer")),
        },
    };
    let limit = match obj.get("limit") {
        None => None,
        Some(v) => match v.as_i64() {
            Some(0) => None,
            Some(n) if n > 0 => Some(n),
            _ => return Err(invalid("`limit` must be a non-negative integer")),
        },
    };
    Ok(Query::Find {
        filter: doc_or_empty(obj, "filter")?,
        projection: opt_doc(obj, "projection")?,
        sort: opt_doc(obj, "sort")?,
        limit,
        skip,
    })
}

fn parse_aggregate(q: &Value) -> Result<Query> {
    let stages = q
        .as_array()
        .ok_or_else(|| invalid("aggregate query must be an array of stages"))?;
    if stages.is_empty() {
        return Err(invalid("the pipeline has no stages"));
    }
    let pipeline = stages
        .iter()
        .enumerate()
        .map(|(i, s)| to_document(s, &format!("stage {i}")))
        .collect::<Result<Vec<_>>>()?;
    Ok(Query::Aggregate { pipeline })
}

fn parse_count(q: &Value) -> Result<Query> {
    let obj = object(q, &["filter"], "count")?;
    Ok(Query::Count {
        filter: doc_or_empty(obj, "filter")?,
    })
}

fn parse_distinct(q: &Value) -> Result<Query> {
    let obj = object(q, &["key", "filter"], "distinct")?;
    let key = obj
        .get("key")
        .and_then(Value::as_str)
        .filter(|k| !k.is_empty() && k.len() <= 256)
        .ok_or_else(|| invalid("distinct needs a `key`: a field name like \"city\""))?;
    Ok(Query::Distinct {
        key: key.to_string(),
        filter: doc_or_empty(obj, "filter")?,
    })
}

fn parse_insert(q: &Value) -> Result<Query> {
    let obj = object(q, &["documents", "document"], "insert")?;
    let raw: Vec<&Value> = match (obj.get("documents"), obj.get("document")) {
        (Some(Value::Array(a)), None) => a.iter().collect(),
        (None, Some(d)) => vec![d],
        _ => {
            return Err(invalid(
                "insert needs `documents` (an array) or `document` (one object)",
            ))
        }
    };
    if raw.is_empty() {
        return Err(invalid("insert has no documents"));
    }
    let documents = raw
        .iter()
        .enumerate()
        .map(|(i, d)| to_document(d, &format!("documents[{i}]")))
        .collect::<Result<Vec<_>>>()?;
    Ok(Query::Insert { documents })
}

fn parse_update(q: &Value) -> Result<Query> {
    let obj = object(q, &["filter", "update", "multi", "upsert"], "update")?;
    let update_json = obj
        .get("update")
        .ok_or_else(|| invalid("update query needs an `update`"))?;
    let update = to_bson(update_json, "update")?;
    match &update {
        Bson::Document(_) => {}
        Bson::Array(stages) if stages.iter().all(|s| matches!(s, Bson::Document(_))) => {}
        _ => {
            return Err(invalid(
                "`update` must be a document or an array of pipeline stages",
            ))
        }
    }
    Ok(Query::Update {
        filter: doc_or_empty(obj, "filter")?,
        update,
        multi: opt_bool(obj, "multi")?,
        upsert: opt_bool(obj, "upsert")?,
    })
}

fn parse_list_indexes(q: &Value) -> Result<Query> {
    if q.is_null() || q.as_object().is_some_and(|o| o.is_empty()) {
        Ok(Query::ListIndexes)
    } else {
        Err(invalid("listIndexes takes no query; send {} or null"))
    }
}

fn parse_delete(q: &Value) -> Result<Query> {
    let obj = object(q, &["filter", "multi"], "delete")?;
    Ok(Query::Delete {
        filter: doc_or_empty(obj, "filter")?,
        multi: opt_bool(obj, "multi")?,
    })
}

pub(crate) fn to_bson(v: &Value, what: &str) -> Result<Bson> {
    Bson::try_from(v.clone()).map_err(|e| invalid(format!("{what}: invalid Extended JSON: {e}")))
}

pub(crate) fn to_document(v: &Value, what: &str) -> Result<Document> {
    match to_bson(v, what)? {
        Bson::Document(d) => Ok(d),
        _ => Err(invalid(format!("{what} must be an object"))),
    }
}

/// Walk a BSON value and reject forbidden operators, cross-database references, and
/// excessive nesting anywhere in the tree (so `$expr: {$function: ..}` is caught too).
pub(crate) fn walk(b: &Bson, depth: usize, limits: &Limits) -> Result<()> {
    if depth > limits.max_depth {
        return Err(invalid(format!(
            "nesting is deeper than {} levels",
            limits.max_depth
        )));
    }
    match b {
        Bson::Document(d) => {
            for (k, v) in d {
                if FORBIDDEN.contains(&k.as_str()) {
                    return Err(PlaygroundError::Forbidden(format!(
                        "`{k}` is not allowed in the playground"
                    )));
                }
                if matches!(k.as_str(), "$lookup" | "$graphLookup" | "$unionWith") {
                    check_same_database(k, v)?;
                }
                if k == "$regex" {
                    if let Bson::String(pattern) = v {
                        check_regex(pattern, "", limits)?;
                    }
                }
                walk(v, depth + 1, limits)?;
            }
        }
        Bson::Array(a) => {
            for v in a {
                walk(v, depth + 1, limits)?;
            }
        }
        Bson::RegularExpression(r) => check_regex(&r.pattern, &r.options, limits)?,
        _ => {}
    }
    Ok(())
}

/// Pattern length cap and the flags MongoDB accepts. Backtracking time is bounded by `maxTimeMS`.
fn check_regex(pattern: &str, options: &str, limits: &Limits) -> Result<()> {
    if pattern.chars().count() > limits.max_regex_pattern_len {
        return Err(invalid(format!(
            "a regex is longer than {} characters",
            limits.max_regex_pattern_len
        )));
    }
    if let Some(bad) = options.chars().find(|c| !"imxsu".contains(*c)) {
        return Err(invalid(format!(
            "regex option `{bad}` is not supported; use i, m, x, s or u"
        )));
    }
    Ok(())
}

/// Index options users may set. `storageEngine`, `hidden` and anything else is rejected.
const INDEX_OPTIONS: &[&str] = &[
    "name",
    "unique",
    "sparse",
    "expireAfterSeconds",
    "partialFilterExpression",
    "collation",
    "weights",
    "wildcardProjection",
    "default_language",
    "language_override",
    "textIndexVersion",
    "2dsphereIndexVersion",
];

fn prepare_indexes(
    requested: &BTreeMap<String, Vec<IndexSpec>>,
    collections: &BTreeMap<String, Vec<Document>>,
    limits: &Limits,
) -> Result<BTreeMap<String, Vec<Document>>> {
    let mut out = BTreeMap::new();
    for (coll, specs) in requested {
        if !collections.contains_key(coll) {
            return Err(invalid(format!(
                "indexes: collection `{coll}` is not defined in the config"
            )));
        }
        if specs.len() > limits.max_indexes_per_collection {
            return Err(invalid(format!(
                "`{coll}` has {} indexes, the limit is {} per collection (the automatic _id index does not count)",
                specs.len(),
                limits.max_indexes_per_collection
            )));
        }
        let prepared = specs
            .iter()
            .enumerate()
            .map(|(i, s)| prepare_index(coll, i, s, limits))
            .collect::<Result<Vec<_>>>()?;
        if !prepared.is_empty() {
            out.insert(coll.clone(), prepared);
        }
    }
    Ok(out)
}

pub(crate) fn prepare_index(
    coll: &str,
    i: usize,
    spec: &IndexSpec,
    limits: &Limits,
) -> Result<Document> {
    let at = format!("indexes.{coll}[{i}]");
    if spec.key.is_empty() || spec.key.len() > limits.max_index_key_fields {
        return Err(invalid(format!(
            "{at}: an index needs 1 to {} key fields",
            limits.max_index_key_fields
        )));
    }
    let mut key = Document::new();
    let mut default_name = Vec::new();
    for (field, dir) in &spec.key {
        let wildcard = field == "$**" || field.ends_with(".$**");
        if field.is_empty()
            || field.len() > 256
            || field.contains('\0')
            || (field.starts_with('$') && !wildcard)
        {
            return Err(invalid(format!("{at}: invalid key field `{field}`")));
        }
        let (value, label) = match dir {
            Value::Number(n) if n.as_i64() == Some(1) => (Bson::Int32(1), "1".to_string()),
            Value::Number(n) if n.as_i64() == Some(-1) => (Bson::Int32(-1), "-1".to_string()),
            Value::String(s) if matches!(s.as_str(), "text" | "2dsphere" | "hashed") => {
                (Bson::String(s.clone()), s.clone())
            }
            _ => {
                return Err(invalid(format!(
                    "{at}: key `{field}` must be 1, -1, \"text\", \"2dsphere\" or \"hashed\""
                )))
            }
        };
        key.insert(field.clone(), value);
        default_name.push(format!("{field}_{label}"));
    }

    let mut out = doc! { "key": key };
    let mut name = None;
    for (opt, v) in &spec.options {
        if !INDEX_OPTIONS.contains(&opt.as_str()) {
            return Err(invalid(format!(
                "{at}: option `{opt}` is not allowed; allowed: {}",
                INDEX_OPTIONS.join(", ")
            )));
        }
        match opt.as_str() {
            "name" => {
                let n = v.as_str().filter(|n| {
                    !n.is_empty()
                        && n.len() <= 64
                        && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                });
                name = Some(
                    n.ok_or_else(|| {
                        invalid(format!("{at}: `name` must be 1-64 letters, digits or `_`"))
                    })?
                    .to_string(),
                );
            }
            "unique" | "sparse" => {
                let b = v
                    .as_bool()
                    .ok_or_else(|| invalid(format!("{at}: `{opt}` must be true or false")))?;
                out.insert(opt.clone(), b);
            }
            "expireAfterSeconds" | "textIndexVersion" | "2dsphereIndexVersion" => {
                let n = v
                    .as_i64()
                    .filter(|n| (0..=31_536_000).contains(n))
                    .ok_or_else(|| {
                        invalid(format!(
                            "{at}: `{opt}` must be an integer between 0 and 31536000"
                        ))
                    })?;
                out.insert(opt.clone(), n);
            }
            "default_language" | "language_override" => {
                let s = v
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 64)
                    .ok_or_else(|| invalid(format!("{at}: `{opt}` must be a short string")))?;
                out.insert(opt.clone(), s);
            }
            // partialFilterExpression, collation, weights, wildcardProjection
            _ => {
                let d = to_document(v, &format!("{at}: {opt}"))?;
                walk(&Bson::Document(d.clone()), 0, limits)?;
                out.insert(opt.clone(), d);
            }
        }
    }
    let name = name.unwrap_or_else(|| {
        let generated = default_name.join("_");
        if generated.len() <= 64 {
            generated
        } else {
            format!("idx_{i}")
        }
    });
    out.insert("name", name);
    Ok(out)
}

/// `$lookup`/`$graphLookup` take `from`, `$unionWith` takes a name or `{coll, pipeline}`.
/// The document forms of `from` can name another database, so only plain strings pass.
fn check_same_database(op: &str, v: &Bson) -> Result<()> {
    let cross_db = || {
        PlaygroundError::Forbidden(format!(
            "`{op}` may only reference collections in the config"
        ))
    };
    match v {
        Bson::String(_) if op == "$unionWith" => Ok(()),
        Bson::Document(d) => {
            if d.contains_key("db") {
                return Err(cross_db());
            }
            let key = if op == "$unionWith" { "coll" } else { "from" };
            match d.get(key) {
                Some(Bson::String(_)) | None => Ok(()),
                Some(_) => Err(cross_db()),
            }
        }
        _ => Ok(()),
    }
}

pub(crate) fn invalid(msg: impl Into<String>) -> PlaygroundError {
    PlaygroundError::Invalid(msg.into())
}

/// What validation reports on success: `{valid, mode, collection, documentCounts, ...}`.
pub fn prepare_ok_summary(p: &Prepared) -> Value {
    let counts: Map<String, Value> = p
        .collections
        .iter()
        .map(|(k, v)| (k.clone(), Value::from(v.len())))
        .collect();
    let mut out = serde_json::json!({
        "valid": true,
        "mode": p.query.mode().as_str(),
        "collection": p.collection,
        "documentCounts": counts,
        "indexes": p.indexes.values().map(Vec::len).sum::<usize>(),
        "searchIndexes": p.search_indexes.len(),
    });
    if let Some(db) = &p.sample_db {
        out["sampleDatabase"] = Value::from(db.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn req(mode: &str, query: Value) -> RunRequest {
        serde_json::from_value(json!({
            "mode": mode,
            "collections": { "people": [ {"name": "Ada", "age": 36}, {"name": "Linus", "age": 55} ] },
            "collection": "people",
            "query": query,
        }))
        .unwrap()
    }

    fn limits() -> Limits {
        Limits::default()
    }

    #[test]
    fn find_ok() {
        let r = req("find", json!({"filter": {"age": {"$gt": 40}}, "limit": 5}));
        let p = prepare(&r, &limits()).unwrap();
        assert!(matches!(p.query, Query::Find { limit: Some(5), .. }));
    }

    #[test]
    fn find_limit_is_not_capped() {
        let r = req("find", json!({"filter": {}, "limit": 100000}));
        match prepare(&r, &limits()).unwrap().query {
            Query::Find { limit, .. } => assert_eq!(limit, Some(100000)),
            _ => panic!(),
        }
        let r = req("find", json!({"filter": {}}));
        match prepare(&r, &limits()).unwrap().query {
            Query::Find { limit, .. } => assert_eq!(limit, None),
            _ => panic!(),
        }
        let r = req("find", json!({"filter": {}, "limit": -1}));
        assert!(matches!(
            prepare(&r, &limits()),
            Err(PlaygroundError::Invalid(_))
        ));
    }

    #[test]
    fn extended_json_is_parsed() {
        let r = req(
            "find",
            json!({"filter": {"_id": {"$oid": "507f1f77bcf86cd799439011"}}}),
        );
        let p = prepare(&r, &limits()).unwrap();
        match p.query {
            Query::Find { filter, .. } => {
                assert!(matches!(filter.get("_id"), Some(Bson::ObjectId(_))))
            }
            _ => panic!(),
        }
    }

    #[test]
    fn rejects_where_and_function() {
        let r = req("find", json!({"filter": {"$where": "sleep(10000)"}}));
        assert!(matches!(
            prepare(&r, &limits()),
            Err(PlaygroundError::Forbidden(_))
        ));
        let r = req(
            "aggregate",
            json!([{"$match": {"$expr": {"$function": {"body": "x", "args": [], "lang": "js"}}}}]),
        );
        assert!(matches!(
            prepare(&r, &limits()),
            Err(PlaygroundError::Forbidden(_))
        ));
    }

    #[test]
    fn rejects_out_and_merge() {
        for stage in ["$out", "$merge"] {
            let r = req("aggregate", json!([{ stage: "other" }]));
            assert!(matches!(
                prepare(&r, &limits()),
                Err(PlaygroundError::Forbidden(_))
            ));
        }
    }

    #[test]
    fn rejects_cross_db_lookup() {
        let r = req(
            "aggregate",
            json!([{"$lookup": {"from": {"db": "admin", "coll": "system.users"}, "as": "x", "pipeline": []}}]),
        );
        assert!(matches!(
            prepare(&r, &limits()),
            Err(PlaygroundError::Forbidden(_))
        ));
        let r = req(
            "aggregate",
            json!([{"$unionWith": {"db": "admin", "coll": "x"}}]),
        );
        assert!(matches!(
            prepare(&r, &limits()),
            Err(PlaygroundError::Forbidden(_))
        ));
    }

    #[test]
    fn allows_same_db_lookup() {
        let r = req(
            "aggregate",
            json!([{"$lookup": {"from": "people", "localField": "a", "foreignField": "b", "as": "x"}}, {"$unionWith": "people"}]),
        );
        assert!(prepare(&r, &limits()).is_ok());
    }

    #[test]
    fn rejects_unknown_collection_and_bad_names() {
        let mut r = req("find", json!({}));
        r.collection = "nope".into();
        assert!(matches!(
            prepare(&r, &limits()),
            Err(PlaygroundError::Invalid(_))
        ));
        let mut r = req("find", json!({}));
        r.collections.insert("system.users".into(), vec![]);
        assert!(matches!(
            prepare(&r, &limits()),
            Err(PlaygroundError::Invalid(_))
        ));
    }

    #[test]
    fn enforces_counts_and_size() {
        let mut r = req("find", json!({}));
        r.collections
            .insert("big".into(), (0..257).map(|i| json!({"i": i})).collect());
        assert!(matches!(
            prepare(&r, &limits()),
            Err(PlaygroundError::Invalid(_))
        ));

        let mut r = req("find", json!({}));
        for i in 0..9 {
            r.collections.insert(format!("c{i}"), vec![]);
        }
        assert!(matches!(
            prepare(&r, &limits()),
            Err(PlaygroundError::Invalid(_))
        ));

        let mut r = req("find", json!({}));
        r.collections.insert(
            "blob".into(),
            vec![json!({"s": "x".repeat(33 * 1024 * 1024)})],
        );
        assert!(matches!(
            prepare(&r, &limits()),
            Err(PlaygroundError::Invalid(_))
        ));
    }

    #[test]
    fn enforces_depth() {
        let mut v = json!(1);
        for _ in 0..40 {
            v = json!({ "a": v });
        }
        let r = req("find", json!({"filter": v}));
        assert!(matches!(
            prepare(&r, &limits()),
            Err(PlaygroundError::Invalid(_))
        ));
    }

    #[test]
    fn update_shapes() {
        let r = req(
            "update",
            json!({"filter": {"name": "Ada"}, "update": {"$set": {"age": 37}}}),
        );
        assert!(prepare(&r, &limits()).is_ok());
        let r = req(
            "update",
            json!({"filter": {}, "update": [{"$set": {"x": 1}}], "multi": true, "upsert": true}),
        );
        assert!(matches!(
            prepare(&r, &limits()).unwrap().query,
            Query::Update {
                multi: true,
                upsert: true,
                ..
            }
        ));
        let r = req("update", json!({"filter": {}, "update": 5}));
        assert!(prepare(&r, &limits()).is_err());
        let r = req("update", json!({"filter": {}}));
        assert!(prepare(&r, &limits()).is_err());
    }

    #[test]
    fn aggregate_must_be_array() {
        let r = req("aggregate", json!({"$match": {}}));
        assert!(prepare(&r, &limits()).is_err());
    }

    #[test]
    fn count_distinct_delete_shapes() {
        assert!(prepare(
            &req("count", json!({"filter": {"age": {"$gt": 1}}})),
            &limits()
        )
        .is_ok());
        assert!(prepare(&req("count", json!({})), &limits()).is_ok());
        assert!(prepare(&req("count", json!({"bogus": 1})), &limits()).is_err());
        assert!(prepare(&req("distinct", json!({"key": "name"})), &limits()).is_ok());
        assert!(prepare(&req("distinct", json!({"filter": {}})), &limits()).is_err());
        let p = prepare(
            &req("delete", json!({"filter": {"name": "Ada"}, "multi": true})),
            &limits(),
        )
        .unwrap();
        assert!(matches!(p.query, Query::Delete { multi: true, .. }));
        assert_eq!(p.query.mode(), Mode::Delete);
    }

    #[test]
    fn insert_shapes_and_limit() {
        assert!(prepare(
            &req("insert", json!({"documents": [{"a": 1}, {"a": 2}]})),
            &limits()
        )
        .is_ok());
        assert!(prepare(&req("insert", json!({"document": {"a": 1}})), &limits()).is_ok());
        assert!(prepare(&req("insert", json!({"documents": []})), &limits()).is_err());
        assert!(prepare(&req("insert", json!({"documents": [1]})), &limits()).is_err());
        assert!(prepare(&req("insert", json!({})), &limits()).is_err());
        // 2 documents are already seeded; 255 more would make 257.
        let many: Vec<_> = (0..255).map(|i| json!({"i": i})).collect();
        assert!(prepare(&req("insert", json!({"documents": many})), &limits()).is_err());
        let ok: Vec<_> = (0..254).map(|i| json!({"i": i})).collect();
        assert!(prepare(&req("insert", json!({"documents": ok})), &limits()).is_ok());
    }

    #[test]
    fn insert_cannot_be_explained_but_others_can() {
        let mut r = req("insert", json!({"document": {"a": 1}}));
        r.explain = Some(Verbosity::ExecutionStats);
        assert!(matches!(
            prepare(&r, &limits()),
            Err(PlaygroundError::Invalid(_))
        ));
        let mut r = req("find", json!({}));
        r.explain = Some(Verbosity::QueryPlanner);
        assert_eq!(
            prepare(&r, &limits()).unwrap().explain,
            Some(Verbosity::QueryPlanner)
        );
    }

    #[test]
    fn forbidden_operators_are_caught_in_every_mode() {
        let r = req("delete", json!({"filter": {"$where": "1"}}));
        assert!(matches!(
            prepare(&r, &limits()),
            Err(PlaygroundError::Forbidden(_))
        ));
        let r = req("insert", json!({"document": {"a": {"$function": {}}}}));
        assert!(matches!(
            prepare(&r, &limits()),
            Err(PlaygroundError::Forbidden(_))
        ));
        let r = req("count", json!({"filter": {"$where": "1"}}));
        assert!(matches!(
            prepare(&r, &limits()),
            Err(PlaygroundError::Forbidden(_))
        ));
    }

    fn with_indexes(indexes: Value) -> RunRequest {
        let mut r = req("find", json!({"filter": {}}));
        r.indexes = serde_json::from_value(indexes).unwrap();
        r
    }

    #[test]
    fn regex_literals_in_text_queries_become_bson_regexes() {
        let r: RunRequest = serde_json::from_value(json!({
            "mode": "find",
            "collections": r#"{"people": [{"name": "Ada"}]}"#,
            "collection": "people",
            "query": r#"{"filter": {"name": /^a/i}}  // find Ada"#,
        }))
        .unwrap();
        match prepare(&r, &limits()).unwrap().query {
            Query::Find { filter, .. } => match filter.get("name") {
                Some(Bson::RegularExpression(re)) => {
                    assert_eq!((re.pattern.as_str(), re.options.as_str()), ("^a", "i"))
                }
                other => panic!("expected a regex, got {other:?}"),
            },
            _ => panic!(),
        }
    }

    #[test]
    fn text_collections_can_hold_regex_literals_too() {
        let r: RunRequest = serde_json::from_value(json!({
            "mode": "find",
            "collections": "{\"people\": [{\"pattern\": /x/}]}",
            "collection": "people",
            "query": {"filter": {}},
        }))
        .unwrap();
        assert!(prepare(&r, &limits()).is_ok());
    }

    #[test]
    fn bad_relaxed_text_fails_to_deserialize_with_a_message() {
        let err = serde_json::from_value::<RunRequest>(json!({
            "mode": "find", "collections": {"people": []}, "collection": "people",
            "query": "{\"filter\": /x/g}",
        }))
        .unwrap_err()
        .to_string();
        assert!(err.contains("query") && err.contains("`g`"), "{err}");
    }

    #[test]
    fn regex_limits_apply_to_every_form() {
        let long = "x".repeat(257);
        for q in [
            json!({"filter": {"name": {"$regex": long}}}),
            json!({"filter": {"name": {"$regularExpression": {"pattern": long, "options": ""}}}}),
        ] {
            assert!(matches!(
                prepare(&req("find", q), &limits()),
                Err(PlaygroundError::Invalid(_))
            ));
        }
        let r = req(
            "find",
            json!({"filter": {"name": {"$regularExpression": {"pattern": "a", "options": "g"}}}}),
        );
        assert!(prepare(&r, &limits()).is_err());
        let r = req(
            "find",
            json!({"filter": {"name": {"$regex": "^a", "$options": "i"}}}),
        );
        assert!(prepare(&r, &limits()).is_ok());
    }

    #[test]
    fn index_specs_are_validated_and_named() {
        let r = with_indexes(json!({"people": [
            {"key": {"age": 1, "name": -1}},
            {"key": {"bio": "text"}, "options": {"default_language": "english"}},
            {"key": {"city": 1}, "options": {"unique": true, "name": "city_u", "partialFilterExpression": {"age": {"$gt": 1}}}}
        ]}));
        let p = prepare(&r, &limits()).unwrap();
        let specs = &p.indexes["people"];
        assert_eq!(specs[0].get_str("name").unwrap(), "age_1_name_-1");
        assert_eq!(specs[1].get_str("name").unwrap(), "bio_text");
        assert_eq!(specs[2].get_str("name").unwrap(), "city_u");
        assert!(specs[2].get_bool("unique").unwrap());
    }

    #[test]
    fn at_most_three_indexes_per_collection() {
        let three = json!({"people": [{"key": {"a": 1}}, {"key": {"b": 1}}, {"key": {"c": 1}}]});
        assert!(prepare(&with_indexes(three), &limits()).is_ok());
        let four = json!({"people": [{"key": {"a": 1}}, {"key": {"b": 1}}, {"key": {"c": 1}}, {"key": {"d": 1}}]});
        let err = prepare(&with_indexes(four), &limits())
            .unwrap_err()
            .to_string();
        assert!(err.contains("limit is 3"), "{err}");
    }

    #[test]
    fn bad_index_specs_are_rejected() {
        let bad = [
            json!({"people": [{"key": {}}]}),
            json!({"people": [{"key": {"a": 1, "b": 1, "c": 1, "d": 1, "e": 1}}]}),
            json!({"people": [{"key": {"a": 2}}]}),
            json!({"people": [{"key": {"a": "bogus"}}]}),
            json!({"people": [{"key": {"$where": 1}}]}),
            json!({"people": [{"key": {"a": 1}, "options": {"storageEngine": {}}}]}),
            json!({"people": [{"key": {"a": 1}, "options": {"hidden": true}}]}),
            json!({"people": [{"key": {"a": 1}, "options": {"name": "bad name!"}}]}),
            json!({"people": [{"key": {"a": 1}, "options": {"unique": "yes"}}]}),
            json!({"people": [{"key": {"a": 1}, "options": {"partialFilterExpression": {"$where": "1"}}}]}),
            json!({"nope": [{"key": {"a": 1}}]}),
        ];
        for b in bad {
            assert!(
                prepare(&with_indexes(b.clone()), &limits()).is_err(),
                "should reject {b}"
            );
        }
        assert!(
            prepare(
                &with_indexes(json!({"people": [{"key": {"$**": 1}}]})),
                &limits()
            )
            .is_ok(),
            "wildcard index"
        );
        assert!(prepare(
            &with_indexes(json!({"people": [{"key": {"loc": "2dsphere"}}]})),
            &limits()
        )
        .is_ok());
    }

    #[test]
    fn indexes_accept_relaxed_text() {
        let r: RunRequest = serde_json::from_value(json!({
            "mode": "find", "collections": {"people": []}, "collection": "people",
            "query": {"filter": {}},
            "indexes": "{\"people\": [{\"key\": {\"age\": 1}}]} // by age",
        }))
        .unwrap();
        assert_eq!(prepare(&r, &limits()).unwrap().indexes["people"].len(), 1);
    }

    #[test]
    fn list_indexes_mode() {
        assert!(prepare(&req("listIndexes", json!({})), &limits()).is_ok());
        assert!(prepare(&req("listIndexes", Value::Null), &limits()).is_ok());
        assert!(prepare(&req("listIndexes", json!({"filter": {}})), &limits()).is_err());
        let mut r = req("listIndexes", json!({}));
        r.explain = Some(Verbosity::QueryPlanner);
        assert!(prepare(&r, &limits()).is_err());
    }

    fn sample_req(mode: &str, query: Value) -> RunRequest {
        serde_json::from_value(json!({
            "mode": mode, "datasource": {"sample": "sample_mflix"},
            "collection": "movies", "query": query,
        }))
        .unwrap()
    }

    #[test]
    fn sample_data_is_read_only_and_validated() {
        let p = prepare(
            &sample_req("find", json!({"filter": {"year": 1999}})),
            &limits(),
        )
        .unwrap();
        assert_eq!(p.sample_db.as_deref(), Some("sample_mflix"));
        assert!(prepare(&sample_req("aggregate", json!([{"$match": {}}])), &limits()).is_ok());
        for mode in ["insert", "update", "delete", "listIndexes"] {
            assert!(
                prepare(&sample_req(mode, json!({})), &limits()).is_err(),
                "{mode} must be rejected"
            );
        }
        // forbidden operators still apply
        let r = sample_req("find", json!({"filter": {"$where": "1"}}));
        assert!(matches!(
            prepare(&r, &limits()),
            Err(PlaygroundError::Forbidden(_))
        ));
        // no seeded data or indexes alongside a sample datasource
        let mut r = sample_req("find", json!({}));
        r.collections.insert("x".into(), vec![]);
        assert!(prepare(&r, &limits()).is_err());
        // only sample_* databases
        for bad in ["admin", "sample_", "sample_UP", "sample_a.b", "local"] {
            let r: RunRequest = serde_json::from_value(json!({
                "mode": "find", "datasource": {"sample": bad}, "collection": "c", "query": {}
            }))
            .unwrap();
            assert!(prepare(&r, &limits()).is_err(), "{bad} must be rejected");
        }
    }

    fn search_req(indexes: Value, query: Value) -> RunRequest {
        let mut r = req("aggregate", query);
        r.search_indexes = serde_json::from_value(indexes).unwrap();
        r
    }

    #[test]
    fn search_index_specs_and_search_stages() {
        let pipeline = json!([{"$search": {"text": {"query": "ada", "path": "name"}}}]);
        let ok = json!({"people": {"definition": {"mappings": {"dynamic": true}}}});
        let p = prepare(&search_req(ok.clone(), pipeline.clone()), &limits()).unwrap();
        let spec = &p.search_indexes["people"][0];
        assert_eq!(
            (spec.get_str("name").unwrap(), spec.get_str("type").unwrap()),
            ("default", "search")
        );

        // $search without a declared index is a clear error
        let err = prepare(&req("aggregate", pipeline.clone()), &limits())
            .unwrap_err()
            .to_string();
        assert!(err.contains("searchIndexes"), "{err}");

        for bad in [
            json!({"people": {"definition": {}}}),
            json!({"people": {"definition": {"fields": []}}}),
            json!({"people": {"name": "bad name", "definition": {"mappings": {}}}}),
            json!({"nope": {"definition": {"mappings": {}}}}),
            json!({"people": {"definition": {"mappings": {"$where": 1}}}}),
        ] {
            assert!(
                prepare(&search_req(bad.clone(), pipeline.clone()), &limits()).is_err(),
                "should reject {bad}"
            );
        }
    }
}
