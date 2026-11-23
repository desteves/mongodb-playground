//! Prebuilt example datasets: a small, read-only database (`sample_playground`) with an Atlas Search
//! index on `movies` and a Vector Search index on `items`, built once so the built-in search examples
//! answer instantly instead of waiting for an index build. Run with `api seed-examples`.
//!
//! The database is named `sample_*`, so the playground's read-only sample path serves it (when the
//! sample-data feature is on). Seeding needs a user that can write and create search indexes; the
//! playground's own users cannot, on purpose.

use mongodb::{
    bson::{doc, Bson, Document},
    Client,
};

use crate::{error::PlaygroundError, runner::wait_for_search_index};

type Result<T> = std::result::Result<T, PlaygroundError>;

const DATA: &str = include_str!("example_data.json");

/// What a seeding run did, for the log.
#[derive(Debug, Clone)]
pub struct SeedReport {
    pub database: String,
    pub collections: Vec<(String, usize)>,
}

/// The database the prebuilt examples read from.
pub fn database_name() -> String {
    serde_json::from_str::<serde_json::Value>(DATA)
        .ok()
        .and_then(|v| v["database"].as_str().map(String::from))
        .unwrap_or_else(|| "sample_playground".into())
}

/// Insert (replacing what is there) the example documents and create each search index if it is
/// missing, waiting until it is queryable. Safe to run again.
pub async fn seed_examples(uri: &str) -> Result<SeedReport> {
    let client = Client::with_uri_str(uri).await?;
    let data: serde_json::Value =
        serde_json::from_str(DATA).map_err(|e| PlaygroundError::Database(e.to_string()))?;
    let db_name = data["database"].as_str().unwrap_or("sample_playground");
    let db = client.database(db_name);
    let mut report = SeedReport {
        database: db_name.to_string(),
        collections: vec![],
    };
    for c in data["collections"].as_array().into_iter().flatten() {
        let name = c["name"].as_str().unwrap_or_default();
        let docs: Vec<Document> = c["documents"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|d| {
                mongodb::bson::to_document(d).map_err(|e| PlaygroundError::Database(e.to_string()))
            })
            .collect::<Result<_>>()?;
        let coll = db.collection::<Document>(name);
        // Replacing documents makes a search index catch up again, so leave a complete collection alone.
        let have = coll.count_documents(doc! {}).await? as usize;
        if have != docs.len() {
            coll.delete_many(doc! {}).await?;
            coll.insert_many(&docs).await?;
        }

        let spec: Document = mongodb::bson::to_document(&c["searchIndex"])
            .map_err(|e| PlaygroundError::Database(e.to_string()))?;
        let index = spec.get_str("name").unwrap_or("default").to_string();
        let existing = db
            .run_command(doc! {
                "aggregate": name,
                "pipeline": [ { "$listSearchIndexes": { "name": &index } } ],
                "cursor": {},
            })
            .await?;
        let has_index = existing
            .get_document("cursor")
            .and_then(|c| c.get_array("firstBatch"))
            .map(|b| !b.is_empty())
            .unwrap_or(false);
        if !has_index {
            db.run_command(doc! { "createSearchIndexes": name, "indexes": [Bson::Document(spec)] })
                .await?;
        }
        wait_for_search_index(&db, name, &index).await?;
        // "Queryable" can be true before the index has taken in the documents: wait until a real query answers.
        if let Some(Bson::Array(probe)) = c["probe"]
            .as_array()
            .map(|_| mongodb::bson::to_bson(&c["probe"]).unwrap_or(Bson::Null))
        {
            wait_for_probe(&db, name, probe).await?;
        }
        report.collections.push((name.to_string(), docs.len()));
    }
    Ok(report)
}

/// Poll an aggregation until it returns a document (up to two minutes).
async fn wait_for_probe(db: &mongodb::Database, coll: &str, pipeline: Vec<Bson>) -> Result<()> {
    for _ in 0..240 {
        let res = db
            .run_command(doc! { "aggregate": coll, "pipeline": pipeline.clone(), "cursor": {} })
            .await?;
        let found = res
            .get_document("cursor")
            .and_then(|c| c.get_array("firstBatch"))
            .map(|b| !b.is_empty())
            .unwrap_or(false);
        if found {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err(PlaygroundError::Database(format!(
        "the search index on `{coll}` did not start answering queries in time"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_is_well_formed() {
        let data: serde_json::Value = serde_json::from_str(DATA).unwrap();
        assert!(database_name().starts_with("sample_"));
        let colls = data["collections"].as_array().unwrap();
        assert_eq!(colls.len(), 2);
        for c in colls {
            assert!(c["documents"].as_array().unwrap().len() >= 10);
            assert!(c["searchIndex"]["definition"].is_object());
        }
        // 3-dimensional vectors, as the index says
        for d in colls[1]["documents"].as_array().unwrap() {
            assert_eq!(d["embedding"].as_array().unwrap().len(), 3);
        }
    }
}
