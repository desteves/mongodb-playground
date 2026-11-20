//! The shell-style input: `db.people.find({age: {$gt: 30}}).sort({age: -1})`, several statements
//! in a row, run in order against the same throwaway database.
//!
//! This module parses the script into [`Statement`]s, validates every argument with the same
//! rules as the structured API (forbidden operators, depth, regex limits, index limits), and
//! prepares the data panel (documents, indexes, search and vector indexes per collection).

use std::collections::{BTreeMap, BTreeSet};

use mongodb::bson::{Bson, Document};
use serde_json::{Map, Value};

use crate::{
    error::PlaygroundError,
    limits::Limits,
    request::{Datasource, IndexSpec, ScriptRequest, SearchIndexSpec, Verbosity},
    shellvalue::Parser,
    validate::{
        check_collection_name, check_sample_name, invalid, prepare_index, prepare_search_index,
        query_uses_stage, to_bson, to_document, walk, walk_query, Query,
    },
};

type Result<T> = std::result::Result<T, PlaygroundError>;

pub const MAX_STATEMENTS: usize = 25;

#[derive(Debug, Clone)]
pub enum Op {
    Query(Query),
    /// `.explain()` on a find or aggregate: the plan, no results.
    Explain(Verbosity, Query),
    CreateIndex(Document),
    DropIndex(String),
    /// A `createSearchIndexes` spec (type `search` or `vectorSearch`).
    CreateSearchIndex(Document),
}

impl Op {
    pub fn label(&self) -> &'static str {
        match self {
            Op::Query(q) => q.mode().as_str(),
            Op::Explain(..) => "explain",
            Op::CreateIndex(_) => "createIndex",
            Op::DropIndex(_) => "dropIndex",
            Op::CreateSearchIndex(_) => "createSearchIndex",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Statement {
    pub index: usize,
    pub line: usize,
    /// The statement as typed.
    pub text: String,
    pub collection: String,
    pub op: Op,
}

#[derive(Debug, Clone)]
pub struct PreparedScript {
    pub collections: BTreeMap<String, Vec<Document>>,
    /// `createIndexes` specs declared in the data panel.
    pub indexes: BTreeMap<String, Vec<Document>>,
    /// Search and vector search index specs declared in the data panel.
    pub search_indexes: BTreeMap<String, Vec<Document>>,
    pub statements: Vec<Statement>,
    pub explain_all: Option<Verbosity>,
    pub sample_db: Option<String>,
}

impl PreparedScript {
    /// Does this run build a search or vector index (slower, and limited in number)?
    pub fn uses_search(&self) -> bool {
        !self.search_indexes.is_empty()
            || self
                .statements
                .iter()
                .any(|s| matches!(s.op, Op::CreateSearchIndex(_)))
    }
}

struct Call {
    name: String,
    args: Vec<Value>,
}

/// What the script has declared so far, to enforce per-collection limits as it reads.
#[derive(Default)]
struct Tracker {
    indexes: BTreeMap<String, usize>,
    search: BTreeSet<String>,
    vector: BTreeSet<String>,
    inserted: BTreeMap<String, usize>,
}

pub fn prepare_script(req: &ScriptRequest, limits: &Limits) -> Result<PreparedScript> {
    let size = req.script.len()
        + req
            .config
            .collections
            .values()
            .map(|c| {
                serde_json::to_vec(&c.documents)
                    .map(|v| v.len())
                    .unwrap_or(usize::MAX)
            })
            .fold(0usize, |a, b| a.saturating_add(b));
    if size > limits.max_payload_bytes {
        return Err(invalid(format!(
            "payload is {size} bytes, the limit is {}",
            limits.max_payload_bytes
        )));
    }

    let sample = match &req.datasource {
        Some(Datasource::Sample(db)) => {
            check_sample_name(db)?;
            if !req.config.collections.is_empty() {
                return Err(invalid(
                    "sample data is read-only: clear the data panel, or drop `datasource`",
                ));
            }
            Some(db.clone())
        }
        None => None,
    };

    // ---- the data panel -------------------------------------------------------------------
    let mut tracker = Tracker::default();
    let mut collections = BTreeMap::new();
    let mut indexes = BTreeMap::new();
    let mut search_indexes: BTreeMap<String, Vec<Document>> = BTreeMap::new();
    for (name, setup) in &req.config.collections {
        check_collection_name(name)?;
        if setup.documents.len() > limits.max_docs_per_collection {
            return Err(invalid(format!(
                "collection `{name}` has {} documents, the limit is {}",
                setup.documents.len(),
                limits.max_docs_per_collection
            )));
        }
        let mut docs = Vec::with_capacity(setup.documents.len());
        for (i, d) in setup.documents.iter().enumerate() {
            let doc = to_document(d, &format!("{name}[{i}]"))?;
            walk(&Bson::Document(doc.clone()), 0, limits)?;
            docs.push(doc);
        }
        collections.insert(name.clone(), docs);

        if setup.indexes.len() > limits.max_indexes_per_collection {
            return Err(invalid(format!(
                "`{name}` has {} indexes, the limit is {} per collection (the automatic _id index does not count)",
                setup.indexes.len(),
                limits.max_indexes_per_collection
            )));
        }
        let specs = setup
            .indexes
            .iter()
            .enumerate()
            .map(|(i, s)| prepare_index(name, i, s, limits))
            .collect::<Result<Vec<_>>>()?;
        tracker.indexes.insert(name.clone(), specs.len());
        if !specs.is_empty() {
            indexes.insert(name.clone(), specs);
        }

        if setup.search.len() > 1 {
            return Err(invalid(format!(
                "`{name}` has {} search indexes, the limit is 1 per collection",
                setup.search.len()
            )));
        }
        if setup.vector.len() > 1 {
            return Err(invalid(format!(
                "`{name}` has {} vector search indexes, the limit is 1 per collection",
                setup.vector.len()
            )));
        }
        let mut specs = Vec::new();
        if let Some(s) = setup.search.first() {
            specs.push(prepare_search_index(name, s, limits, false)?);
            tracker.search.insert(name.clone());
        }
        if let Some(s) = setup.vector.first() {
            specs.push(prepare_search_index(name, s, limits, true)?);
            tracker.vector.insert(name.clone());
        }
        if !specs.is_empty() {
            search_indexes.insert(name.clone(), specs);
        }
        tracker.inserted.insert(name.clone(), setup.documents.len());
    }

    // ---- the script -----------------------------------------------------------------------
    let statements = parse_script(&req.script, limits, sample.is_some(), &mut tracker)?;
    if statements.is_empty() {
        return Err(invalid(
            "the script has no statements; try db.collection.find({})",
        ));
    }

    if sample.is_some() {
        for s in &statements {
            let read = match &s.op {
                Op::Query(q) | Op::Explain(_, q) => {
                    matches!(
                        q,
                        Query::Find { .. }
                            | Query::Aggregate { .. }
                            | Query::Count { .. }
                            | Query::Distinct { .. }
                    )
                }
                _ => false,
            };
            if !read {
                return Err(invalid(format!(
                    "statement {} (line {}): sample data is read-only; use find, aggregate, count or distinct",
                    s.index + 1,
                    s.line
                )));
            }
        }
    } else {
        let names: BTreeSet<&str> = collections
            .keys()
            .map(String::as_str)
            .chain(statements.iter().map(|s| s.collection.as_str()))
            .collect();
        if names.len() > limits.max_collections {
            return Err(invalid(format!(
                "at most {} collections are allowed; this uses {}",
                limits.max_collections,
                names.len()
            )));
        }
        for (coll, n) in &tracker.inserted {
            if *n > limits.max_docs_per_collection {
                return Err(invalid(format!(
                    "`{coll}` would hold {n} documents after the inserts, the limit is {} per collection",
                    limits.max_docs_per_collection
                )));
            }
        }
    }

    Ok(PreparedScript {
        collections,
        indexes,
        search_indexes,
        statements,
        explain_all: req.explain,
        sample_db: sample,
    })
}

fn parse_script(
    src: &str,
    limits: &Limits,
    allow_unindexed_search: bool,
    tracker: &mut Tracker,
) -> Result<Vec<Statement>> {
    let mut p = Parser::new(src, "script");
    let mut out: Vec<Statement> = Vec::new();
    loop {
        while p.eat(';')? {}
        if p.peek()?.is_none() {
            break;
        }
        if out.len() >= MAX_STATEMENTS {
            return p.fail(
                p.pos,
                format!("at most {MAX_STATEMENTS} statements are allowed"),
            );
        }
        let start = p.pos;
        let line = p.line(start);
        let (collection, calls, end) = statement(&mut p)?;
        // Up to the closing paren: not the comments and blank lines that follow.
        let text = p.text(start, end);
        let n = out.len() + 1;
        let ctx = |e: PlaygroundError| match e {
            PlaygroundError::Invalid(m) => {
                PlaygroundError::Invalid(format!("statement {n} (line {line}): {m}"))
            }
            PlaygroundError::Forbidden(m) => {
                PlaygroundError::Forbidden(format!("statement {n} (line {line}): {m}"))
            }
            other => other,
        };
        check_collection_name(&collection).map_err(ctx)?;
        let op =
            compile(&collection, calls, limits, allow_unindexed_search, tracker).map_err(ctx)?;
        out.push(Statement {
            index: out.len(),
            line,
            text,
            collection,
            op,
        });
    }
    Ok(out)
}

/// `db.people.find(...).sort(...)`, `db["people"]...`, `db.getCollection("people")...`
fn statement(p: &mut Parser) -> Result<(String, Vec<Call>, usize)> {
    let at = p.pos;
    if p.ident()?.as_deref() != Some("db") {
        return p.fail(at, "statements start with `db`, like db.people.find({...})");
    }
    let collection = if p.eat('[')? {
        let s = p.string()?;
        p.expect(']')?;
        s
    } else {
        p.expect('.')?;
        let Some(name) = p.ident()? else {
            return p.fail(p.pos, "expected a collection name after `db.`");
        };
        if name == "getCollection" {
            p.expect('(')?;
            let s = p.string()?;
            p.expect(')')?;
            s
        } else {
            name
        }
    };
    let mut calls = Vec::new();
    let mut end = p.pos;
    while p.eat('.')? {
        let Some(name) = p.ident()? else {
            return p.fail(p.pos, "expected a method name");
        };
        p.expect('(')?;
        let args = p.args()?;
        calls.push(Call { name, args });
        end = p.pos;
    }
    if calls.is_empty() {
        return p.fail(
            p.pos,
            format!("expected a method call, like db.{collection}.find({{}})"),
        );
    }
    Ok((collection, calls, end))
}

// ----- argument helpers ------------------------------------------------------------------------

fn arity(name: &str, a: &[Value], min: usize, max: usize) -> Result<()> {
    if a.len() < min || a.len() > max {
        let want = if min == max {
            min.to_string()
        } else {
            format!("{min} to {max}")
        };
        return Err(invalid(format!(
            "{name}() takes {want} argument(s), got {}",
            a.len()
        )));
    }
    Ok(())
}

fn doc(a: &[Value], i: usize, what: &str) -> Result<Document> {
    match a.get(i) {
        Some(v) => to_document(v, what),
        None => Err(invalid(format!("missing {what}"))),
    }
}

fn opt_doc(a: &[Value], i: usize, what: &str) -> Result<Option<Document>> {
    a.get(i).map(|v| to_document(v, what)).transpose()
}

fn int(a: &[Value], i: usize, what: &str) -> Result<i64> {
    a.get(i)
        .and_then(Value::as_i64)
        .filter(|n| *n >= 0)
        .ok_or_else(|| invalid(format!("{what} must be a non-negative integer")))
}

fn text(a: &[Value], i: usize, what: &str) -> Result<String> {
    a.get(i)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| invalid(format!("{what} must be a string")))
}

fn map(a: &[Value], i: usize, what: &str) -> Result<Map<String, Value>> {
    match a.get(i) {
        Some(Value::Object(m)) => Ok(m.clone()),
        _ => Err(invalid(format!("{what} must be an object"))),
    }
}

/// Options objects: only the listed keys are understood; anything else is an error, not silently ignored.
fn options(a: &[Value], i: usize, name: &str, allowed: &[&str]) -> Result<Map<String, Value>> {
    let m = match a.get(i) {
        None => return Ok(Map::new()),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err(invalid(format!("{name}() options must be an object"))),
    };
    if let Some(bad) = m.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(invalid(format!(
            "{name}() option `{bad}` is not supported; supported: {}",
            allowed.join(", ")
        )));
    }
    Ok(m)
}

fn verbosity(args: &[Value]) -> Result<Verbosity> {
    match args.first() {
        None => Ok(Verbosity::ExecutionStats),
        Some(Value::String(s)) => match s.as_str() {
            "queryPlanner" => Ok(Verbosity::QueryPlanner),
            "executionStats" => Ok(Verbosity::ExecutionStats),
            "allPlansExecution" => Ok(Verbosity::AllPlansExecution),
            other => Err(invalid(format!(
                "explain(\"{other}\"): use queryPlanner, executionStats or allPlansExecution"
            ))),
        },
        Some(_) => Err(invalid("explain() takes a verbosity string")),
    }
}

/// Cursor methods that change nothing here (the playground always returns every document).
const IGNORED: &[&str] = &[
    "toArray",
    "pretty",
    "batchSize",
    "maxTimeMS",
    "allowDiskUse",
    "comment",
];

fn compile(
    coll: &str,
    calls: Vec<Call>,
    limits: &Limits,
    allow_unindexed_search: bool,
    tracker: &mut Tracker,
) -> Result<Op> {
    let (first, rest) = calls
        .split_first()
        .expect("a statement has at least one call");
    let a = &first.args;
    let name = first.name.as_str();

    // Chains after most operations may only be `.explain()` (plus harmless cursor calls).
    let generic_chain = |rest: &[Call]| -> Result<Option<Verbosity>> {
        let mut explain = None;
        for c in rest {
            match c.name.as_str() {
                "explain" => explain = Some(verbosity(&c.args)?),
                n if IGNORED.contains(&n) => {}
                n => return Err(invalid(format!("`.{n}()` cannot follow {name}()"))),
            }
        }
        Ok(explain)
    };
    let wrap = |q: Query, explain: Option<Verbosity>| -> Result<Op> {
        match explain {
            Some(_) if matches!(q, Query::Insert { .. } | Query::ListIndexes) => {
                Err(invalid(format!("{name}() cannot be explained")))
            }
            Some(v) => Ok(Op::Explain(v, q)),
            None => Ok(Op::Query(q)),
        }
    };

    let op = match name {
        "find" | "findOne" => {
            arity(name, a, 0, 2)?;
            let filter = opt_doc(a, 0, "the filter")?.unwrap_or_default();
            let mut projection = opt_doc(a, 1, "the projection")?;
            let (mut sort, mut skip, mut explain, mut count) = (None, None, None, false);
            let mut limit = if name == "findOne" { Some(1) } else { None };
            for c in rest {
                match c.name.as_str() {
                    "sort" => sort = Some(doc(&c.args, 0, "the sort")?),
                    "limit" => limit = Some(int(&c.args, 0, "the limit")?).filter(|n| *n > 0),
                    "skip" => skip = Some(int(&c.args, 0, "the skip")?).filter(|n| *n > 0),
                    "projection" | "project" => projection = Some(doc(&c.args, 0, "the projection")?),
                    "explain" => explain = Some(verbosity(&c.args)?),
                    "count" => count = true,
                    n if IGNORED.contains(&n) => {}
                    n => {
                        return Err(invalid(format!(
                            "`.{n}()` is not supported on find(); supported: sort, limit, skip, projection, count, explain, toArray"
                        )))
                    }
                }
            }
            let q = if count { Query::Count { filter } } else { Query::Find { filter, projection, sort, limit, skip } };
            wrap(q, explain)?
        }
        "aggregate" => {
            arity(name, a, 1, 2)?;
            options(a, 1, name, &["allowDiskUse", "cursor"])?;
            let Some(Value::Array(stages)) = a.first() else {
                return Err(invalid("aggregate() takes an array of stages: aggregate([{$match: {}}])"));
            };
            if stages.is_empty() {
                return Err(invalid("the pipeline has no stages"));
            }
            let pipeline = stages
                .iter()
                .enumerate()
                .map(|(i, s)| to_document(s, &format!("stage {i}")))
                .collect::<Result<Vec<_>>>()?;
            let q = Query::Aggregate { pipeline };
            if query_uses_stage(&q, &["$search", "$searchMeta"]) && !allow_unindexed_search && !tracker.search.contains(coll) {
                return Err(invalid(format!(
                    "$search needs a search index on `{coll}`: add one under searchIndexes in the data panel, or run db.{coll}.createSearchIndex(...) first"
                )));
            }
            if query_uses_stage(&q, &["$vectorSearch"]) && !allow_unindexed_search && !tracker.vector.contains(coll) {
                return Err(invalid(format!(
                    "$vectorSearch needs a vector search index on `{coll}`: add one under vectorSearchIndexes in the data panel, or run db.{coll}.createSearchIndex(name, \"vectorSearch\", {{fields: [...]}}) first"
                )));
            }
            wrap(q, generic_chain(rest)?)?
        }
        "countDocuments" | "count" => {
            arity(name, a, 0, 2)?;
            wrap(Query::Count { filter: opt_doc(a, 0, "the filter")?.unwrap_or_default() }, generic_chain(rest)?)?
        }
        "estimatedDocumentCount" => {
            arity(name, a, 0, 1)?;
            wrap(Query::Count { filter: Document::new() }, generic_chain(rest)?)?
        }
        "distinct" => {
            arity(name, a, 1, 2)?;
            let key = text(a, 0, "the key")?;
            if key.is_empty() || key.len() > 256 {
                return Err(invalid("distinct() needs a field name"));
            }
            wrap(Query::Distinct { key, filter: opt_doc(a, 1, "the filter")?.unwrap_or_default() }, generic_chain(rest)?)?
        }
        "insertOne" | "insertMany" => {
            arity(name, a, 1, 2)?;
            options(a, 1, name, &["ordered"])?;
            let documents = if name == "insertOne" {
                vec![doc(a, 0, "the document")?]
            } else {
                match a.first() {
                    Some(Value::Array(items)) if !items.is_empty() => items
                        .iter()
                        .enumerate()
                        .map(|(i, d)| to_document(d, &format!("documents[{i}]")))
                        .collect::<Result<Vec<_>>>()?,
                    _ => return Err(invalid("insertMany() takes a non-empty array of documents")),
                }
            };
            *tracker.inserted.entry(coll.to_string()).or_default() += documents.len();
            wrap(Query::Insert { documents }, generic_chain(rest)?)?
        }
        "updateOne" | "updateMany" | "replaceOne" => {
            arity(name, a, 2, 3)?;
            let opts = options(a, 2, name, &["upsert"])?;
            let update = to_bson(&a[1], "the update")?;
            let ok = match &update {
                Bson::Document(_) => true,
                Bson::Array(stages) => name != "replaceOne" && stages.iter().all(|s| matches!(s, Bson::Document(_))),
                _ => false,
            };
            if !ok {
                return Err(invalid(format!("{name}() needs an update document{}", if name == "replaceOne" { "" } else { " or a pipeline array" })));
            }
            let upsert = match opts.get("upsert") {
                None => false,
                Some(v) => v.as_bool().ok_or_else(|| invalid("`upsert` must be true or false"))?,
            };
            wrap(
                Query::Update { filter: doc(a, 0, "the filter")?, update, multi: name == "updateMany", upsert },
                generic_chain(rest)?,
            )?
        }
        "deleteOne" | "deleteMany" => {
            arity(name, a, 1, 2)?;
            wrap(Query::Delete { filter: doc(a, 0, "the filter")?, multi: name == "deleteMany" }, generic_chain(rest)?)?
        }
        "getIndexes" => {
            arity(name, a, 0, 0)?;
            wrap(Query::ListIndexes, generic_chain(rest)?)?
        }
        "createIndex" => {
            arity(name, a, 1, 2)?;
            if !rest.is_empty() {
                return Err(invalid("createIndex() cannot be chained"));
            }
            let n = tracker.indexes.entry(coll.to_string()).or_default();
            if *n >= limits.max_indexes_per_collection {
                return Err(invalid(format!(
                    "`{coll}` already has {n} indexes, the limit is {} per collection (the automatic _id index does not count)",
                    limits.max_indexes_per_collection
                )));
            }
            let spec = IndexSpec { key: map(a, 0, "the index keys")?, options: options_any(a, 1, name)? };
            let built = prepare_index(coll, *n, &spec, limits)?;
            *n += 1;
            Op::CreateIndex(built)
        }
        "dropIndex" => {
            arity(name, a, 1, 1)?;
            if !rest.is_empty() {
                return Err(invalid("dropIndex() cannot be chained"));
            }
            let index = text(a, 0, "the index name")?;
            if index == "_id_" {
                return Err(invalid("the _id index cannot be dropped"));
            }
            let n = tracker.indexes.entry(coll.to_string()).or_default();
            *n = n.saturating_sub(1);
            Op::DropIndex(index)
        }
        "createSearchIndex" => {
            arity(name, a, 1, 3)?;
            if !rest.is_empty() {
                return Err(invalid("createSearchIndex() cannot be chained"));
            }
            let (spec, vector) = search_spec(a)?;
            let taken = if vector { &mut tracker.vector } else { &mut tracker.search };
            if !taken.insert(coll.to_string()) {
                return Err(invalid(format!(
                    "`{coll}` already has a {} index; the limit is 1 per collection",
                    if vector { "vector search" } else { "search" }
                )));
            }
            Op::CreateSearchIndex(prepare_search_index(coll, &spec, limits, vector)?)
        }
        other => {
            return Err(invalid(format!(
                "db.{coll}.{other}() is not supported. Supported: find, findOne, aggregate, countDocuments, estimatedDocumentCount, distinct, insertOne, insertMany, updateOne, updateMany, replaceOne, deleteOne, deleteMany, createIndex, dropIndex, getIndexes, createSearchIndex"
            )))
        }
    };

    if let Op::Query(q) | Op::Explain(_, q) = &op {
        walk_query(q, limits)?;
    }
    Ok(op)
}

fn options_any(a: &[Value], i: usize, name: &str) -> Result<Map<String, Value>> {
    match a.get(i) {
        None => Ok(Map::new()),
        Some(Value::Object(m)) => Ok(m.clone()),
        Some(_) => Err(invalid(format!("{name}() options must be an object"))),
    }
}

/// `createSearchIndex(definition)`, `(name, definition)`, `(name, "vectorSearch", definition)` or
/// `({name, type, definition})`. Returns the spec and whether it is a vector search index.
fn search_spec(a: &[Value]) -> Result<(SearchIndexSpec, bool)> {
    let (mut name, mut kind) = (None, None);
    let mut rest = a;
    if let Some(Value::String(n)) = rest.first() {
        name = Some(n.clone());
        rest = &rest[1..];
    }
    if let Some(Value::String(t)) = rest.first() {
        kind = Some(t.clone());
        rest = &rest[1..];
    }
    let Some(Value::Object(obj)) = rest.first() else {
        return Err(invalid("createSearchIndex() needs a definition object, for example {mappings: {dynamic: true}}"));
    };
    let definition = match obj.get("definition") {
        Some(Value::Object(d)) => {
            if name.is_none() {
                name = obj.get("name").and_then(Value::as_str).map(str::to_string);
            }
            if kind.is_none() {
                kind = obj.get("type").and_then(Value::as_str).map(str::to_string);
            }
            d.clone()
        }
        _ => obj.clone(),
    };
    let vector = match kind.as_deref() {
        None | Some("search") => false,
        Some("vectorSearch") => true,
        Some(other) => {
            return Err(invalid(format!(
                "index type `{other}` is not supported; use \"search\" or \"vectorSearch\""
            )))
        }
    };
    Ok((SearchIndexSpec { name, definition }, vector))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn script(src: &str) -> Result<PreparedScript> {
        let req: ScriptRequest = serde_json::from_value(
            json!({ "script": src, "config": {"people": [{"name": "Ada", "age": 36}]} }),
        )
        .unwrap();
        prepare_script(&req, &Limits::default())
    }

    fn ops(src: &str) -> Vec<&'static str> {
        script(src)
            .unwrap_or_else(|e| panic!("{src}: {e}"))
            .statements
            .iter()
            .map(|s| s.op.label())
            .collect()
    }

    #[test]
    fn every_supported_method_parses() {
        let src = r#"
            db.people.find({age: {$gt: 30}}, {name: 1}).sort({age: -1}).limit(5).skip(1).toArray()
            db.people.findOne({name: 'Ada'})
            db.people.aggregate([{$match: {}}, {$group: {_id: '$city', n: {$sum: 1}}}])
            db.people.countDocuments({age: {$gte: 1}});
            db.people.estimatedDocumentCount()
            db.people.distinct('city')
            db.people.insertOne({name: 'Tim'})
            db.people.insertMany([{name: 'Bo'}, {name: 'Cy'}])
            db.people.updateOne({name: 'Ada'}, {$set: {age: 37}}, {upsert: true})
            db.people.updateMany({}, [{$set: {x: 1}}])
            db.people.replaceOne({name: 'Bo'}, {name: 'Bob'})
            db.people.deleteOne({name: 'Cy'})
            db.people.deleteMany({age: {$lt: 0}})
            db.people.createIndex({age: 1}, {unique: false})
            db.people.getIndexes()
            db.people.dropIndex('age_1')
        "#;
        assert_eq!(
            ops(src),
            [
                "find",
                "find",
                "aggregate",
                "count",
                "count",
                "distinct",
                "insert",
                "insert",
                "update",
                "update",
                "update",
                "delete",
                "delete",
                "createIndex",
                "listIndexes",
                "dropIndex"
            ]
        );
    }

    #[test]
    fn statements_need_no_semicolons_and_chains_may_span_lines() {
        let p = script(
            "db.people.find({})\n  .sort({age: 1})\n  .limit(1)\n\n// next\ndb.people.count()",
        )
        .unwrap();
        assert_eq!(p.statements.len(), 2);
        assert_eq!(p.statements[0].line, 1);
        assert_eq!(p.statements[1].line, 6);
        assert!(p.statements[0].text.contains(".limit(1)"));
        assert!(matches!(
            &p.statements[0].op,
            Op::Query(Query::Find { limit: Some(1), .. })
        ));
    }

    #[test]
    fn other_collection_syntaxes() {
        assert_eq!(
            script(r#"db["orders"].find(); db.getCollection('my_items').find()"#)
                .unwrap()
                .statements[1]
                .collection,
            "my_items"
        );
    }

    #[test]
    fn explain_chains() {
        let p = script("db.people.find({}).explain('queryPlanner'); db.people.aggregate([{$match: {}}]).explain()").unwrap();
        assert!(matches!(
            &p.statements[0].op,
            Op::Explain(Verbosity::QueryPlanner, _)
        ));
        assert!(matches!(
            &p.statements[1].op,
            Op::Explain(Verbosity::ExecutionStats, _)
        ));
        assert!(script("db.people.insertOne({}).explain()").is_err());
        assert!(script("db.people.find().explain('nope')").is_err());
    }

    #[test]
    fn find_count_chain_becomes_a_count() {
        assert!(matches!(
            &script("db.people.find({age: 1}).count()")
                .unwrap()
                .statements[0]
                .op,
            Op::Query(Query::Count { .. })
        ));
    }

    #[test]
    fn shell_values_work_inside_statements() {
        let p = script("db.people.insertOne({_id: ObjectId('507f1f77bcf86cd799439011'), at: ISODate('2024-01-01T00:00:00Z')}); db.people.find({name: /^a/i})").unwrap();
        assert_eq!(p.statements.len(), 2);
    }

    #[test]
    fn errors_say_which_statement_and_line() {
        let e = script("db.people.find({})\ndb.people.bogus({})")
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("statement 2") && e.contains("line 2") && e.contains("bogus"),
            "{e}"
        );
        let e = script("db.people.find({$where: 'x'})").unwrap_err();
        assert!(matches!(e, PlaygroundError::Forbidden(_)));
        assert!(e.to_string().contains("statement 1"));
        assert!(script("people.find({})")
            .unwrap_err()
            .to_string()
            .contains("start with `db`"));
        assert!(script("db.people.find({a: })").is_err());
        assert!(script("db.people.find()  .skip(-1)").is_err());
        assert!(script("db.people.updateOne({}, 5)").is_err());
        assert!(
            script("db.people.updateOne({}, {$set: {a: 1}}, {arrayFilters: []})")
                .unwrap_err()
                .to_string()
                .contains("arrayFilters")
        );
        assert!(script("")
            .unwrap_err()
            .to_string()
            .contains("no statements"));
        assert!(script("db.people").is_err());
    }

    #[test]
    fn limits_apply() {
        let many = (0..26)
            .map(|_| "db.people.find()")
            .collect::<Vec<_>>()
            .join("\n");
        assert!(script(&many)
            .unwrap_err()
            .to_string()
            .contains("at most 25"));
        let nine = (0..9)
            .map(|i| format!("db.c{i}.find()"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(script(&nine)
            .unwrap_err()
            .to_string()
            .contains("at most 8 collections"));
        let docs = (0..256)
            .map(|i| format!("{{i: {i}}}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(script(&format!("db.people.insertMany([{docs}])"))
            .unwrap_err()
            .to_string()
            .contains("documents after the inserts"));
    }

    #[test]
    fn at_most_three_indexes_per_collection_across_panel_and_statements() {
        let ok = "db.people.createIndex({a: 1}); db.people.createIndex({b: 1}); db.people.createIndex({c: 1})";
        assert_eq!(ops(ok).len(), 3);
        let e = script(&format!("{ok}; db.people.createIndex({{d: 1}})"))
            .unwrap_err()
            .to_string();
        assert!(e.contains("limit is 3"), "{e}");
        // dropping frees a slot
        assert!(script(&format!(
            "{ok}; db.people.dropIndex('c_1'); db.people.createIndex({{d: 1}})"
        ))
        .is_ok());
        // indexes from the data panel count too
        let req: ScriptRequest = serde_json::from_value(json!({
            "script": "db.people.createIndex({z: 1})",
            "config": {"people": {"documents": [], "indexes": [{"key": {"a": 1}}, {"key": {"b": 1}}, {"key": {"c": 1}}]}}
        })).unwrap();
        assert!(prepare_script(&req, &Limits::default()).is_err());
    }

    #[test]
    fn data_panel_per_collection_setup() {
        let req: ScriptRequest = serde_json::from_value(json!({
            "script": "db.movies.aggregate([{$search: {text: {query: 'x', path: 'plot'}}}])",
            "config": {
                "movies": {
                    "documents": [{"plot": "x", "embedding": [0.1, 0.2, 0.3]}],
                    "indexes": [{"key": {"plot": 1}}],
                    "searchIndexes": [{"definition": {"mappings": {"dynamic": true}}}],
                    "vectorSearchIndexes": [{"definition": {"fields": [{"type": "vector", "path": "embedding", "numDimensions": 3, "similarity": "cosine"}]}}]
                }
            }
        })).unwrap();
        let p = prepare_script(&req, &Limits::default()).unwrap();
        assert_eq!(p.indexes["movies"].len(), 1);
        let kinds: Vec<_> = p.search_indexes["movies"]
            .iter()
            .map(|d| d.get_str("type").unwrap().to_string())
            .collect();
        assert_eq!(kinds, ["search", "vectorSearch"]);
        assert!(p.uses_search());
    }

    #[test]
    fn search_stages_need_their_index() {
        assert!(
            script("db.people.aggregate([{$search: {text: {query: 'a', path: 'name'}}}])")
                .unwrap_err()
                .to_string()
                .contains("search index")
        );
        assert!(script("db.people.aggregate([{$vectorSearch: {index: 'v', path: 'e', queryVector: [1], limit: 1, numCandidates: 5}}])").unwrap_err().to_string().contains("vector search index"));
        let ok = "db.people.createSearchIndex({mappings: {dynamic: true}}); db.people.aggregate([{$search: {text: {query: 'a', path: 'name'}}}])";
        assert!(script(ok).is_ok());
        let e = script("db.people.createSearchIndex({mappings: {dynamic: true}}); db.people.createSearchIndex('two', {mappings: {}})").unwrap_err();
        assert!(e.to_string().contains("1 per collection"), "{e}");
        let v = "db.people.createSearchIndex('vec', 'vectorSearch', {fields: [{type: 'vector', path: 'e', numDimensions: 3, similarity: 'cosine'}]})";
        assert!(script(v).is_ok());
        assert!(
            script("db.people.createSearchIndex('vec', 'vectorSearch', {fields: []})").is_err()
        );
        assert!(script("db.people.createSearchIndex('x', 'bogus', {mappings: {}})").is_err());
        let spec = "db.people.createSearchIndex({name: 'n', type: 'vectorSearch', definition: {fields: [{type: 'vector', path: 'e', numDimensions: 3, similarity: 'dotProduct'}]}})";
        assert!(script(spec).is_ok());
    }

    #[test]
    fn sample_scripts_are_read_only() {
        let sample = |src: &str| -> Result<PreparedScript> {
            let req: ScriptRequest = serde_json::from_value(
                json!({"script": src, "datasource": {"sample": "sample_mflix"}}),
            )
            .unwrap();
            prepare_script(&req, &Limits::default())
        };
        assert!(sample(
            "db.movies.find({year: 1999}).limit(5); db.movies.aggregate([{$match: {}}]).explain()"
        )
        .is_ok());
        assert!(sample("db.movies.insertOne({})").is_err());
        assert!(sample("db.movies.createIndex({a: 1})").is_err());
        assert!(sample("db.movies.find({$where: 'x'})").is_err());
    }

    #[test]
    fn statement_text_stops_at_the_closing_paren() {
        let p =
            script("db.people.find({}) // trailing note\n\n// a comment line\ndb.people.count()  ")
                .unwrap();
        assert_eq!(p.statements[0].text, "db.people.find({})");
        assert_eq!(p.statements[1].text, "db.people.count()");
    }
}
