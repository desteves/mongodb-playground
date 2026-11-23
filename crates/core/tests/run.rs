//! Integration tests. They need a MongoDB; set `MONGODB_URI` (for example
//! `mongodb://localhost:6310/?directConnection=true`) to run them, otherwise each test returns early.

use playground_core::{Limits, RunRequest, Runner};
use serde_json::{json, Value};

/// Tests share one MongoDB and run in parallel. Every test that runs queries holds a read lock;
/// the leak check takes the write lock, so it only looks at databases when nothing else is in
/// flight (otherwise it would see another test's live `pg_*` database and cry "leak").
static GATE: tokio::sync::RwLock<()> = tokio::sync::RwLock::const_new(());

struct Guarded {
    runner: Runner,
    _gate: tokio::sync::RwLockReadGuard<'static, ()>,
}

impl std::ops::Deref for Guarded {
    type Target = Runner;
    fn deref(&self) -> &Runner {
        &self.runner
    }
}

async fn runner() -> Option<Guarded> {
    let uri = std::env::var("MONGODB_URI").ok()?;
    let gate = GATE.read().await;
    Some(Guarded {
        runner: Runner::connect(&uri, Limits::default()).await.unwrap(),
        _gate: gate,
    })
}

fn req(mode: &str, query: Value) -> RunRequest {
    serde_json::from_value(json!({
        "mode": mode,
        "collections": { "people": [
            {"name": "Ada", "age": 36, "city": "London"},
            {"name": "Linus", "age": 55, "city": "Helsinki"},
            {"name": "Grace", "age": 85, "city": "London"}
        ] },
        "collection": "people",
        "query": query,
    }))
    .unwrap()
}

fn names(results: &[Value]) -> Vec<&str> {
    results.iter().filter_map(|d| d["name"].as_str()).collect()
}

#[tokio::test]
async fn find_filters_and_projects() {
    let Some(r) = runner().await else { return };
    let out = r
        .run(&req(
            "find",
            json!({"filter": {"age": {"$gt": 60}}, "projection": {"_id": 0, "name": 1}}),
        ))
        .await
        .unwrap();
    assert_eq!(out.results, vec![json!({"name": "Grace"})]);
}

#[tokio::test]
async fn aggregate_groups() {
    let Some(r) = runner().await else { return };
    let out = r
        .run(&req(
            "aggregate",
            json!([{"$group": {"_id": null, "n": {"$sum": 1}}}]),
        ))
        .await
        .unwrap();
    assert_eq!(out.results, vec![json!({"_id": null, "n": 3})]);
}

#[tokio::test]
async fn count_and_distinct() {
    let Some(r) = runner().await else { return };
    let out = r
        .run(&req("count", json!({"filter": {"city": "London"}})))
        .await
        .unwrap();
    assert_eq!(out.results, vec![json!({"count": 2})]);

    let out = r
        .run(&req("distinct", json!({"key": "city"})))
        .await
        .unwrap();
    let mut cities: Vec<_> = out
        .results
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    cities.sort();
    assert_eq!(cities, ["Helsinki", "London"]);
}

#[tokio::test]
async fn insert_adds_documents_and_returns_the_collection() {
    let Some(r) = runner().await else { return };
    let out = r
        .run(&req(
            "insert",
            json!({"documents": [{"name": "Tim", "age": 30}, {"name": "Barbara", "age": 40}]}),
        ))
        .await
        .unwrap();
    assert_eq!(out.write.unwrap().inserted, Some(2));
    assert_eq!(out.results.len(), 5);
    assert!(names(&out.results).contains(&"Tim"));
}

#[tokio::test]
async fn insert_duplicate_id_is_a_database_error() {
    let Some(r) = runner().await else { return };
    let mut q = req("insert", json!({"documents": [{"_id": 1}, {"_id": 1}]}));
    q.collections.insert("people".into(), vec![]);
    let err = r.run(&q).await.unwrap_err();
    assert_eq!(err.code(), "database_error");
}

#[tokio::test]
async fn update_modifies_and_reports_counts() {
    let Some(r) = runner().await else { return };
    let out = r
        .run(&req(
            "update",
            json!({"filter": {"name": "Ada"}, "update": {"$set": {"age": 37}}}),
        ))
        .await
        .unwrap();
    let w = out.write.unwrap();
    assert_eq!(
        (w.matched, w.modified, w.upserted),
        (Some(1), Some(1), Some(0))
    );
    assert!(out
        .results
        .iter()
        .any(|d| d["name"] == "Ada" && d["age"] == 37));
}

#[tokio::test]
async fn update_upsert_creates_a_document() {
    let Some(r) = runner().await else { return };
    let out = r
        .run(&req(
            "update",
            json!({"filter": {"name": "Zed"}, "update": {"$set": {"age": 1}}, "upsert": true}),
        ))
        .await
        .unwrap();
    let w = out.write.unwrap();
    assert_eq!((w.matched, w.upserted), (Some(0), Some(1)));
    assert!(names(&out.results).contains(&"Zed"));
}

#[tokio::test]
async fn update_without_operators_replaces_the_document() {
    let Some(r) = runner().await else { return };
    let out = r
        .run(&req(
            "update",
            json!({"filter": {"name": "Ada"}, "update": {"name": "Ada L.", "title": "Countess"}}),
        ))
        .await
        .unwrap();
    let doc = out
        .results
        .iter()
        .find(|d| d["name"] == "Ada L.")
        .expect("replaced");
    assert_eq!(doc["title"], "Countess");
    assert!(doc.get("age").is_none());
}

#[tokio::test]
async fn delete_one_and_many() {
    let Some(r) = runner().await else { return };
    let out = r
        .run(&req("delete", json!({"filter": {"city": "London"}})))
        .await
        .unwrap();
    assert_eq!(
        out.write.unwrap().deleted,
        Some(1),
        "multi defaults to false"
    );
    assert_eq!(out.results.len(), 2);

    let out = r
        .run(&req(
            "delete",
            json!({"filter": {"city": "London"}, "multi": true}),
        ))
        .await
        .unwrap();
    assert_eq!(out.write.unwrap().deleted, Some(2));
    assert_eq!(names(&out.results), ["Linus"]);
}

#[tokio::test]
async fn explain_reports_a_collection_scan() {
    let Some(r) = runner().await else { return };
    let out = r
        .explain(&req("find", json!({"filter": {"age": {"$gt": 40}}})))
        .await
        .unwrap();
    assert!(out.results.is_empty());
    let e = out.explain.unwrap();
    assert!(e.summary.collscan, "summary: {:?}", e.summary);
    assert_eq!(e.summary.returned, Some(2));
    assert!(
        e.raw.get("serverInfo").is_none(),
        "host details must be scrubbed"
    );
}

#[tokio::test]
async fn explain_works_for_every_explainable_mode() {
    let Some(r) = runner().await else { return };
    let cases = [
        ("find", json!({"filter": {}})),
        (
            "aggregate",
            json!([{"$match": {"age": {"$gt": 1}}}, {"$group": {"_id": "$city"}}]),
        ),
        ("count", json!({"filter": {"age": {"$gt": 1}}})),
        ("distinct", json!({"key": "city"})),
        (
            "update",
            json!({"filter": {"name": "Ada"}, "update": {"$set": {"x": 1}}}),
        ),
        ("delete", json!({"filter": {"name": "Ada"}})),
    ];
    for (mode, q) in cases {
        let out = r
            .explain(&req(mode, q))
            .await
            .unwrap_or_else(|e| panic!("{mode}: {e}"));
        let e = out.explain.unwrap();
        assert!(
            !e.summary.stages.is_empty(),
            "{mode}: no stages in {}",
            e.raw
        );
    }
}

#[tokio::test]
async fn explaining_a_write_does_not_perform_it() {
    let Some(r) = runner().await else { return };
    let mut q = req("delete", json!({"filter": {}, "multi": true}));
    q.explain = Some(playground_core::Verbosity::ExecutionStats);
    // run + explain: the plan comes from explain (no write), then the delete really runs.
    let out = r.run(&q).await.unwrap();
    assert!(out.explain.is_some());
    assert_eq!(out.write.unwrap().deleted, Some(3));
    assert!(out.results.is_empty());
}

#[tokio::test]
async fn bad_operator_is_a_database_error() {
    let Some(r) = runner().await else { return };
    let err = r
        .run(&req("find", json!({"filter": {"age": {"$nope": 1}}})))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "database_error");
}

#[tokio::test]
async fn results_are_not_capped_and_span_batches() {
    let Some(r) = runner().await else { return };
    // 256 documents is the per-collection limit; the first batch is 101, so this needs getMore.
    let docs: Vec<_> = (0..256).map(|i| json!({"i": i})).collect();
    let request: RunRequest = serde_json::from_value(json!({
        "mode": "find",
        "collections": { "n": docs },
        "collection": "n",
        "query": { "filter": {} },
    }))
    .unwrap();
    assert_eq!(r.run(&request).await.unwrap().results.len(), 256);
}

#[tokio::test]
async fn temp_databases_are_dropped() {
    let Some(uri) = std::env::var("MONGODB_URI").ok() else {
        return;
    };
    let _exclusive = GATE.write().await;
    let r = Runner::connect(&uri, Limits::default()).await.unwrap();
    for _ in 0..3 {
        let _ = r.run(&req("find", json!({"filter": {}}))).await;
    }
    let _ = r.run(&req("find", json!({"filter": {"$nope": 1}}))).await;
    let _ = r.explain(&req("find", json!({"filter": {}}))).await;
    let client = mongodb::Client::with_uri_str(&uri).await.unwrap();
    let names = client.list_database_names().await.unwrap();
    assert!(
        !names.iter().any(|n| n.starts_with("pg_")),
        "leaked databases: {names:?}"
    );
}

fn with_indexes(mut r: RunRequest, indexes: Value) -> RunRequest {
    r.indexes = serde_json::from_value(indexes).unwrap();
    r
}

#[tokio::test]
async fn regex_literal_and_operator_forms_agree() {
    let Some(r) = runner().await else { return };
    let literal: RunRequest = serde_json::from_value(json!({
        "mode": "find",
        "collections": r#"{"people": [{"name": "Ada"}, {"name": "ada lovelace"}, {"name": "Linus"}]}"#,
        "collection": "people",
        "query": r#"{"filter": {"name": /^ada/i}, "sort": {"name": 1}}"#,
    }))
    .unwrap();
    let a = r.run(&literal).await.unwrap().results;
    assert_eq!(names(&a), ["Ada", "ada lovelace"]);

    let mut operator = literal.clone();
    operator.query =
        json!({"filter": {"name": {"$regex": "^ada", "$options": "i"}}, "sort": {"name": 1}});
    let b = r.run(&operator).await.unwrap().results;
    // Compare names: each run seeds fresh documents, so the generated _ids differ.
    assert_eq!(names(&a), names(&b));
}

#[tokio::test]
async fn an_index_turns_a_collection_scan_into_an_index_scan() {
    let Some(r) = runner().await else { return };
    let q = req("find", json!({"filter": {"age": 36}}));
    let without = r.explain(&q).await.unwrap().explain.unwrap().summary;
    assert!(
        without.collscan && without.index_used.is_none(),
        "{without:?}"
    );

    let q = with_indexes(q, json!({"people": [{"key": {"age": 1}}]}));
    let with = r.explain(&q).await.unwrap().explain.unwrap().summary;
    assert!(!with.collscan, "{with:?}");
    assert_eq!(with.index_used.as_deref(), Some("age_1"));
    assert_eq!(with.docs_examined, Some(1));
}

#[tokio::test]
async fn list_indexes_shows_declared_indexes() {
    let Some(r) = runner().await else { return };
    let q = with_indexes(
        req("listIndexes", json!({})),
        json!({"people": [{"key": {"age": 1}}, {"key": {"city": 1, "name": -1}, "options": {"name": "by_city"}}]}),
    );
    let out = r.run(&q).await.unwrap();
    let mut found: Vec<_> = out
        .results
        .iter()
        .map(|i| i["name"].as_str().unwrap().to_string())
        .collect();
    found.sort();
    assert_eq!(found, ["_id_", "age_1", "by_city"]);
}

#[tokio::test]
async fn unique_index_violation_is_a_database_error() {
    let Some(r) = runner().await else { return };
    let q = with_indexes(
        req("find", json!({"filter": {}})),
        json!({"people": [{"key": {"city": 1}, "options": {"unique": true}}]}),
    );
    // two people share the city "London"
    let err = r.run(&q).await.unwrap_err();
    assert_eq!(err.code(), "database_error", "{err}");
}

#[tokio::test]
async fn text_index_supports_text_search() {
    let Some(r) = runner().await else { return };
    let q: RunRequest = serde_json::from_value(json!({
        "mode": "find",
        "collections": {"posts": [{"t": "mongodb is fun"}, {"t": "rust is fast"}]},
        "collection": "posts",
        "query": {"filter": {"$text": {"$search": "rust"}}},
        "indexes": {"posts": [{"key": {"t": "text"}}]},
    }))
    .unwrap();
    let out = r.run(&q).await.unwrap();
    assert_eq!(out.results.len(), 1);
    assert_eq!(out.results[0]["t"], "rust is fast");
}

// ----- share links, stats, sample data, search, sweep -----------------------------------------

async fn raw_client() -> Option<mongodb::Client> {
    let uri = std::env::var("MONGODB_URI").ok()?;
    Some(playground_core::make_client(&uri).await.unwrap())
}

fn shared(query: &str) -> playground_core::SharedPlayground {
    serde_json::from_value(json!({
        "mode": "find",
        "collection": "people",
        "collections": r#"{"people": [{"name": "Ada"}]}"#,
        "query": query,
    }))
    .unwrap()
}

#[tokio::test]
async fn share_round_trip_keeps_the_text_exactly_and_dedups() {
    let Some(client) = raw_client().await else {
        return;
    };
    let shares = playground_core::Shares::new(client.clone(), "pg_test_shares");
    shares.ensure_indexes().await.unwrap();
    let limits = Limits::default();

    let p = shared(r#"{"filter": {"name": /^a/i}}  // as typed"#);
    let id = shares.save(&p, &limits).await.unwrap();
    assert_eq!(id.len(), 36, "links are UUIDs: {id}");
    let again = shares.save(&p, &limits).await.unwrap();
    assert_ne!(again, id, "sharing twice makes two independent links");
    assert_eq!(
        shares.load(&again).await.unwrap().playground,
        p,
        "and both open the same playground"
    );

    let loaded = shares.load(&id).await.unwrap();
    assert_eq!(loaded.playground, p, "regex literal and comment survive");
    assert!(loaded.created_at_ms > 0);

    assert_eq!(
        shares.load("doesnotexist").await.unwrap_err().code(),
        "not_found"
    );
    assert_eq!(shares.load("bad/id").await.unwrap_err().code(), "not_found");

    let mut bad = shared("{}");
    bad.query = json!({"filter": {"$where": "1"}});
    assert_eq!(
        shares.save(&bad, &limits).await.unwrap_err().code(),
        "forbidden"
    );

    client
        .database("pg_test_shares")
        .run_command(mongodb::bson::doc! {"dropDatabase": 1})
        .await
        .unwrap();
}

#[tokio::test]
async fn stats_events_are_written_without_query_text() {
    use mongodb::bson::doc;
    let Some(client) = raw_client().await else {
        return;
    };
    let db = "pg_test_platform";
    let stats = playground_core::Stats::start(client.clone(), db, "test");
    stats.record(playground_core::stats::Event {
        kind: "run".into(),
        client: "agent".into(),
        mode: Some("find".into()),
        dur_ms: Some(7),
        ok: true,
        rows: Some(2),
        ..Default::default()
    });
    stats.flush().await;
    let res = client
        .database(db)
        .run_command(doc! {"find": "events", "filter": {}})
        .await
        .unwrap();
    let batch = res
        .get_document("cursor")
        .unwrap()
        .get_array("firstBatch")
        .unwrap();
    assert_eq!(batch.len(), 1);
    let ev = batch[0].as_document().unwrap();
    assert_eq!(ev.get_str("type").unwrap(), "run");
    assert_eq!(
        ev.get_document("meta").unwrap().get_str("client").unwrap(),
        "agent"
    );
    playground_core::stats::rollup(&client, db, 24)
        .await
        .unwrap();
    let rolled = client
        .database(db)
        .run_command(doc! {"count": "daily_rollups"})
        .await
        .unwrap();
    assert!(rolled.get_i32("n").unwrap_or(0) >= 1 || rolled.get_i64("n").unwrap_or(0) >= 1);
    client
        .database(db)
        .run_command(doc! {"dropDatabase": 1})
        .await
        .unwrap();
}

#[tokio::test]
async fn sample_data_is_read_only_capped_and_never_dropped() {
    use mongodb::bson::{doc, Bson};
    let Some(client) = raw_client().await else {
        return;
    };
    let db = client.database("sample_pgtest");
    let docs: Vec<Bson> = (0..1200)
        .map(|i| Bson::Document(doc! {"i": i, "even": i % 2 == 0}))
        .collect();
    db.run_command(doc! {"insert": "nums", "documents": docs})
        .await
        .unwrap();

    let r = runner().await.unwrap();
    let sample = |mode: &str, query: Value| -> RunRequest {
        serde_json::from_value(json!({
            "mode": mode, "datasource": {"sample": "sample_pgtest"}, "collection": "nums", "query": query
        }))
        .unwrap()
    };

    let out = r.run(&sample("find", json!({"filter": {}}))).await.unwrap();
    assert_eq!(out.results.len(), 1000, "capped at 1000");
    assert_eq!(out.truncated, Some(true));
    let out = r
        .run(&sample("find", json!({"filter": {"i": {"$lt": 5}}})))
        .await
        .unwrap();
    assert_eq!((out.results.len(), out.truncated), (5, None));
    let out = r
        .run(&sample("count", json!({"filter": {"even": true}})))
        .await
        .unwrap();
    assert_eq!(out.results, vec![json!({"count": 600})]);
    let out = r
        .run(&sample(
            "aggregate",
            json!([{"$match": {"even": true}}, {"$group": {"_id": null, "n": {"$sum": 1}}}]),
        ))
        .await
        .unwrap();
    assert_eq!(out.results[0]["n"], 600);
    let out = r
        .explain(&sample("find", json!({"filter": {"i": 3}})))
        .await
        .unwrap();
    assert!(out.explain.unwrap().summary.collscan);

    assert!(r.run(&sample("delete", json!({}))).await.is_err());
    let mut bad = sample("find", json!({}));
    bad.datasource = Some(playground_core::Datasource::Sample("sample_nope".into()));
    assert!(r
        .run(&bad)
        .await
        .unwrap_err()
        .to_string()
        .contains("unknown sample database"));

    assert!(r
        .sample_databases()
        .await
        .unwrap()
        .contains(&"sample_pgtest".to_string()));
    assert_eq!(
        r.sample_collections("sample_pgtest").await.unwrap(),
        ["nums"]
    );
    assert_eq!(
        r.sample_preview("sample_pgtest", "nums")
            .await
            .unwrap()
            .len(),
        3
    );

    // the sample database is still intact after all those runs
    let count = db.run_command(doc! {"count": "nums"}).await.unwrap();
    assert_eq!(
        count.get_i32("n").unwrap_or(-1) as i64 + count.get_i64("n").unwrap_or(0),
        1200
    );
    db.run_command(doc! {"dropDatabase": 1}).await.unwrap();
}

#[tokio::test]
async fn search_index_runs_when_supported() {
    let Some(r) = runner().await else { return };
    let request: RunRequest = serde_json::from_value(json!({
        "mode": "aggregate",
        "collection": "movies",
        "collections": {"movies": [
            {"title": "The Matrix", "plot": "a hacker learns reality is simulated"},
            {"title": "Toy Story", "plot": "toys come to life"},
            {"title": "Alien", "plot": "a crew meets a deadly creature in space"}
        ]},
        "searchIndexes": {"movies": {"definition": {"mappings": {"dynamic": true}}}},
        "query": [
            {"$search": {"text": {"query": "hacker", "path": "plot"}}},
            {"$project": {"_id": 0, "title": 1}}
        ],
    }))
    .unwrap();
    if !r.search_supported().await {
        // plain mongod: the run is rejected with a clear message
        let err = r.run(&request).await.unwrap_err();
        assert!(
            err.to_string().contains("does not support Atlas Search"),
            "{err}"
        );
        return;
    }
    let out = r.run(&request).await.unwrap();
    assert_eq!(out.results, vec![json!({"title": "The Matrix"})]);
}

#[tokio::test]
async fn sweep_drops_only_stale_playground_databases() {
    use mongodb::bson::doc;
    let Some(client) = raw_client().await else {
        return;
    };
    let r = runner().await.unwrap();
    // names are pg_<unix seconds hex>_<random>: one from 2001 (stale), one from "now"
    let stale = "pg_3b9aca00_stale000test";
    let fresh = format!(
        "pg_{:x}_fresh000test",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
    for name in [stale, fresh.as_str()] {
        client
            .database(name)
            .run_command(doc! {"create": "c"})
            .await
            .unwrap();
    }
    let dropped = r
        .sweep_stale(std::time::Duration::from_secs(600))
        .await
        .unwrap();
    assert!(dropped.contains(&stale.to_string()), "{dropped:?}");
    assert!(!dropped.contains(&fresh), "{dropped:?}");
    let names = client.list_database_names().await.unwrap();
    assert!(names.contains(&fresh) && !names.contains(&stale.to_string()));
    client
        .database(&fresh)
        .run_command(doc! {"dropDatabase": 1})
        .await
        .unwrap();
}

// ----- shell-style scripts ---------------------------------------------------------------------

fn script_req(script: &str, config: Value) -> playground_core::ScriptRequest {
    serde_json::from_value(json!({ "script": script, "config": config })).unwrap()
}

#[tokio::test]
async fn a_script_runs_statements_in_order_against_one_database() {
    let Some(r) = runner().await else { return };
    let out = r
        .run_script(&script_req(
            r#"
            db.people.insertMany([{name: 'Tim', age: 30}, {name: 'Bo', age: 41}])
            db.people.updateMany({age: {$gt: 35}}, {$set: {senior: true}})
            db.people.find({senior: true}, {_id: 0, name: 1}).sort({name: 1})
            db.people.deleteOne({name: 'Tim'})
            db.people.countDocuments({})
            db.people.distinct('name')
            "#,
            json!({"people": [{"name": "Ada", "age": 36}]}),
        ))
        .await
        .unwrap();
    assert!(out.ok, "{:?}", out.error);
    let ops: Vec<_> = out.statements.iter().map(|s| s.op).collect();
    assert_eq!(
        ops,
        ["insert", "update", "find", "delete", "count", "distinct"]
    );
    assert_eq!(out.statements[0].write.as_ref().unwrap().inserted, Some(2));
    assert_eq!(
        out.statements[1].write.as_ref().unwrap().modified,
        Some(2),
        "Ada and Bo"
    );
    assert_eq!(
        out.statements[2].results,
        vec![json!({"name": "Ada"}), json!({"name": "Bo"})]
    );
    assert_eq!(out.statements[3].write.as_ref().unwrap().deleted, Some(1));
    assert_eq!(out.statements[4].results, vec![json!({"count": 2})]);
    let mut names: Vec<_> = out.statements[5]
        .results
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    names.sort();
    assert_eq!(names, ["Ada", "Bo"]);
    assert_eq!(
        out.statements[2].statement,
        "db.people.find({senior: true}, {_id: 0, name: 1}).sort({name: 1})"
    );
}

#[tokio::test]
async fn a_failing_statement_stops_the_script_and_keeps_earlier_output() {
    let Some(r) = runner().await else { return };
    let out = r
        .run_script(&script_req(
            "db.people.countDocuments({}); db.people.find({age: {$nope: 1}}); db.people.countDocuments({})",
            json!([{"name": "Ada"}]),
        ))
        .await
        .unwrap();
    assert!(!out.ok);
    assert_eq!(out.statements.len(), 2, "the third statement never ran");
    assert!(out.statements[0].error.is_none());
    assert_eq!(
        out.statements[1].error.as_ref().unwrap().code,
        "database_error"
    );
    assert_eq!(out.error.as_ref().unwrap().code, "database_error");
}

#[tokio::test]
async fn explain_chains_and_explain_all() {
    let Some(r) = runner().await else { return };
    let cfg = json!({"people": {"documents": [{"age": 36}, {"age": 55}], "indexes": [{"key": {"age": 1}}]}});

    let out = r
        .run_script(&script_req(
            "db.people.find({age: 36}).explain()",
            cfg.clone(),
        ))
        .await
        .unwrap();
    let e = out.statements[0].explain.as_ref().unwrap();
    assert_eq!(e.summary.index_used.as_deref(), Some("age_1"));
    assert!(out.statements[0].results.is_empty());

    // explain-all: reads and writes are explained, not run; inserts still run
    let mut req = script_req(
        "db.people.insertOne({age: 1}); db.people.deleteMany({}); db.people.find({})",
        cfg,
    );
    req.explain = Some(playground_core::Verbosity::ExecutionStats);
    let out = r.run_script(&req).await.unwrap();
    assert!(out.ok, "{:?}", out.error);
    assert!(
        out.statements[0].explain.is_none()
            && out.statements[0].write.as_ref().unwrap().inserted == Some(1)
    );
    assert!(
        out.statements[1].explain.is_some(),
        "deleteMany was explained"
    );
    assert_eq!(
        out.statements[2].explain.as_ref().unwrap().summary.returned,
        Some(3),
        "two seeded + one inserted: nothing was deleted, deleteMany was only explained"
    );
}

#[tokio::test]
async fn index_statements_and_the_runtime_limit() {
    let Some(r) = runner().await else { return };
    let out = r
        .run_script(&script_req(
            "db.people.createIndex({a: 1}); db.people.createIndex({b: 1}, {unique: false}); db.people.getIndexes(); db.people.dropIndex('a_1'); db.people.getIndexes()",
            json!([{"a": 1, "b": 2}]),
        ))
        .await
        .unwrap();
    assert!(out.ok, "{:?}", out.error);
    let names = |i: usize| -> Vec<String> {
        let mut v: Vec<_> = out.statements[i]
            .results
            .iter()
            .map(|d| d["name"].as_str().unwrap().to_string())
            .collect();
        v.sort();
        v
    };
    assert_eq!(names(2), ["_id_", "a_1", "b_1"]);
    assert_eq!(names(4), ["_id_", "b_1"]);
}

#[tokio::test]
async fn the_data_panel_sets_up_documents_and_indexes_per_collection() {
    let Some(r) = runner().await else { return };
    let cfg = json!({
        "people": {"documents": [{"city": "A", "n": 1}, {"city": "B", "n": 2}], "indexes": [{"key": {"city": 1}, "options": {"unique": true}}]},
        "other": [{"x": 1}]
    });
    let out = r
        .run_script(&script_req(
            "db.people.getIndexes(); db.other.countDocuments({})",
            cfg,
        ))
        .await
        .unwrap();
    assert!(out.ok, "{:?}", out.error);
    assert_eq!(out.statements[0].results.len(), 2);
    assert_eq!(out.statements[1].results, vec![json!({"count": 1})]);
    // a unique index over duplicate data fails while the panel is being set up
    let dup = json!({"p": {"documents": [{"c": 1}, {"c": 1}], "indexes": [{"key": {"c": 1}, "options": {"unique": true}}]}});
    let err = r
        .run_script(&script_req("db.p.find()", dup))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "database_error");
}

#[tokio::test]
async fn search_and_vector_search_from_the_data_panel() {
    let Some(r) = runner().await else { return };
    let cfg = json!({"items": {
        "documents": [
            {"name": "red apple", "embedding": [0.9, 0.1, 0.0]},
            {"name": "blue car", "embedding": [0.0, 0.1, 0.9]}
        ],
        "searchIndexes": [{"definition": {"mappings": {"dynamic": true}}}],
        "vectorSearchIndexes": [{"definition": {"fields": [{"type": "vector", "path": "embedding", "numDimensions": 3, "similarity": "cosine"}]}}]
    }});
    let script = r#"
        db.items.aggregate([{$search: {text: {query: 'apple', path: 'name'}}}, {$project: {_id: 0, name: 1}}])
        db.items.aggregate([{$vectorSearch: {index: 'vector_index', path: 'embedding', queryVector: [0.85, 0.15, 0.05], numCandidates: 10, limit: 1}}, {$project: {_id: 0, name: 1}}])
    "#;
    if !r.search_supported().await {
        let err = r.run_script(&script_req(script, cfg)).await.unwrap_err();
        assert!(
            err.to_string().contains("does not support Atlas Search"),
            "{err}"
        );
        return;
    }
    let out = r.run_script(&script_req(script, cfg)).await.unwrap();
    assert!(out.ok, "{:?}", out.error);
    assert_eq!(
        out.statements[0].results,
        vec![json!({"name": "red apple"})]
    );
    assert_eq!(
        out.statements[1].results,
        vec![json!({"name": "red apple"})]
    );
}

#[tokio::test]
async fn sample_data_scripts_are_read_only_and_capped() {
    use mongodb::bson::{doc, Bson};
    let Some(client) = raw_client().await else {
        return;
    };
    let db = client.database("sample_pgscript");
    let docs: Vec<Bson> = (0..1200).map(|i| Bson::Document(doc! {"i": i})).collect();
    db.run_command(doc! {"insert": "nums", "documents": docs})
        .await
        .unwrap();
    let r = runner().await.unwrap();
    let req: playground_core::ScriptRequest = serde_json::from_value(json!({
        "datasource": {"sample": "sample_pgscript"},
        "script": "db.nums.find({}); db.nums.countDocuments({i: {$lt: 10}}); db.nums.find({i: 3}).explain()"
    }))
    .unwrap();
    let out = r.run_script(&req).await.unwrap();
    assert!(out.ok, "{:?}", out.error);
    assert_eq!(out.statements[0].results.len(), 1000);
    assert_eq!(out.statements[0].truncated, Some(true));
    assert_eq!(out.statements[1].results, vec![json!({"count": 10})]);
    assert!(out.statements[2].explain.is_some());
    let write: playground_core::ScriptRequest = serde_json::from_value(
        json!({"datasource": {"sample": "sample_pgscript"}, "script": "db.nums.deleteMany({})"}),
    )
    .unwrap();
    assert!(r.run_script(&write).await.is_err());
    let count = db.run_command(doc! {"count": "nums"}).await.unwrap();
    assert_eq!(
        count.get_i32("n").unwrap_or(-1) as i64 + count.get_i64("n").unwrap_or(0),
        1200
    );
    db.run_command(doc! {"dropDatabase": 1}).await.unwrap();
}

#[tokio::test]
async fn shared_script_playgrounds_round_trip() {
    let Some(client) = raw_client().await else {
        return;
    };
    let shares = playground_core::Shares::new(client.clone(), "pg_test_shares_script");
    let p: playground_core::SharedPlayground = serde_json::from_value(json!({
        "script": "db.collection.find({name: /^a/i})  // as typed",
        "config": "[{name: 'Ada'}]"
    }))
    .unwrap();
    let id = shares.save(&p, &Limits::default()).await.unwrap();
    let loaded = shares.load(&id).await.unwrap();
    assert_eq!(loaded.playground, p, "script and data panel text survive");
    client
        .database("pg_test_shares_script")
        .run_command(mongodb::bson::doc! {"dropDatabase": 1})
        .await
        .unwrap();
}

#[tokio::test]
async fn a_result_explosion_hits_the_byte_ceiling_not_the_servers_memory() {
    let Some(r) = runner().await else { return };
    // 256 documents x 60,000 unwound values: millions of results if nothing stops it.
    let docs: Vec<Value> = (0..256).map(|i| json!({"i": i})).collect();
    let out = r
        .run_script(&script_req(
            "db.c.aggregate([{$addFields: {x: {$range: [0, 60000]}}}, {$unwind: '$x'}])",
            json!({"c": docs}),
        ))
        .await
        .unwrap();
    assert!(!out.ok);
    let e = out.error.unwrap();
    assert!(
        e.message.contains("larger than 64 MB")
            || e.code == "timeout"
            || e.message.contains("time limit"),
        "{e:?}"
    );
}

/// The prebuilt example datasets seed, are queryable at once, and serve the built-in search examples.
#[tokio::test]
async fn prebuilt_example_datasets_are_seeded_and_searchable() {
    let Ok(uri) = std::env::var("MONGODB_URI") else {
        return;
    };
    let Some(r) = runner().await else { return };
    if !r.search_supported().await {
        return;
    }
    let report = playground_core::seed::seed_examples(&uri).await.unwrap();
    assert_eq!(report.database, "sample_playground");
    assert_eq!(report.collections.len(), 2);
    // running it again is harmless
    playground_core::seed::seed_examples(&uri).await.unwrap();
    let req: playground_core::ScriptRequest = serde_json::from_value(json!({
        "datasource": {"sample": "sample_playground"},
        "script": "db.movies.aggregate([{ $search: { text: { query: 'alien', path: ['title', 'plot'] } } }, { $project: { _id: 0, title: 1 } }])\ndb.items.aggregate([{ $vectorSearch: { index: 'vector_index', path: 'embedding', queryVector: [0.85, 0.15, 0.05], numCandidates: 16, limit: 3 } }, { $project: { _id: 0, name: 1 } }])"
    }))
    .unwrap();
    let out = r.run_script(&req).await.unwrap();
    assert!(out.ok, "{:?}", out.error);
    assert!(!out.statements[0].results.is_empty());
    assert_eq!(out.statements[1].results.len(), 3);
}
