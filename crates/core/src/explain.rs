//! Turn MongoDB's explain output into a short summary humans and agents can act on.

use serde::Serialize;
use serde_json::Value;

use crate::request::Verbosity;

#[derive(Debug, Serialize)]
pub struct ExplainOutput {
    /// The server's explain document (host details removed), as relaxed Extended JSON.
    pub raw: Value,
    pub summary: ExplainSummary,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExplainSummary {
    pub verbosity: &'static str,
    /// Stage names of the winning plan, root first (for example `["FETCH", "IXSCAN"]`).
    pub stages: Vec<String>,
    pub index_used: Option<String>,
    pub collscan: bool,
    pub keys_examined: Option<i64>,
    pub docs_examined: Option<i64>,
    pub returned: Option<i64>,
    pub execution_time_ms: Option<i64>,
    pub advice: Vec<String>,
}

pub fn summarize(raw: &Value, verbosity: Verbosity) -> ExplainSummary {
    let winning = find_key(raw, "queryPlanner").and_then(|q| q.get("winningPlan"));
    let mut stages = Vec::new();
    let mut index_used = None;
    if let Some(w) = winning {
        collect_stages(w, &mut stages);
        index_used = find_key(w, "indexName")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    let collscan = stages.iter().any(|s| s == "COLLSCAN");

    let stats = find_key(raw, "executionStats");
    let num = |k: &str| stats.and_then(|s| s.get(k)).and_then(Value::as_i64);
    let mut keys_examined = num("totalKeysExamined");
    let mut docs_examined = num("totalDocsExamined");
    let mut returned = num("nReturned");
    let mut execution_time_ms = num("executionTimeMillis");

    // Atlas Search explains have no queryPlanner/winningPlan: the plan is a pipeline whose stages
    // are `$_internalSearchMongotRemote` (mongot answers) and `$_internalSearchIdLookup` (mongod
    // fetches the matches). Read the stage names and counts from there instead.
    let mut search_query_type = None;
    if stages.is_empty() {
        let pipeline = raw.get("stages").and_then(Value::as_array);
        for stage in pipeline.into_iter().flatten().filter_map(Value::as_object) {
            for (name, body) in stage.iter().filter(|(k, _)| k.starts_with('$')) {
                stages.push(name.clone());
                if name == "$_internalSearchIdLookup" {
                    docs_examined = body.get("totalDocsExamined").and_then(Value::as_i64);
                    keys_examined = body.get("totalKeysExamined").and_then(Value::as_i64);
                }
                if name == "$_internalSearchMongotRemote" {
                    search_query_type = Some(
                        body.pointer("/explain/query/type")
                            .and_then(Value::as_str)
                            .unwrap_or("query")
                            .to_string(),
                    );
                }
            }
            returned = stage.get("nReturned").and_then(Value::as_i64).or(returned);
            execution_time_ms = stage
                .get("executionTimeMillisEstimate")
                .and_then(Value::as_i64)
                .or(execution_time_ms);
        }
    }

    let mut advice = Vec::new();
    if let Some(kind) = &search_query_type {
        advice.push(format!(
            "Atlas Search: mongot answered this as a {kind}, then mongod fetched the matching documents by _id. Search explain output differs from ordinary query plans; explain.raw has per-clause timings."
        ));
    }
    if stats.is_none() && returned.is_none() {
        advice.push(
            "Only the plan was requested; ask for executionStats to see keys and documents examined."
                .to_string(),
        );
    }
    if collscan {
        advice.push(
            "COLLSCAN: every document was read. If the filter or sort is selective, an index on those fields would avoid this."
                .to_string(),
        );
        if docs_examined.is_some_and(|d| d <= 256) {
            advice.push(
                "The collection is tiny, so a collection scan may be the planner's right choice here; judge index choices on a larger sample."
                    .to_string(),
            );
        }
    }
    if stages.iter().any(|s| s == "SORT") {
        advice.push(
            "In-memory SORT: an index matching the sort order would avoid sorting in memory."
                .to_string(),
        );
    }
    if let (Some(d), Some(r)) = (docs_examined, returned) {
        if d >= 50 && d > 10 * r.max(1) {
            advice.push(format!(
                "docsExamined ({d}) is much larger than nReturned ({r}): the query reads many documents it then discards. A more selective index could help."
            ));
        }
        if index_used.is_some() && d == r && d > 0 {
            advice.push(
                "An index was used and the scan is tight (docsExamined equals nReturned)."
                    .to_string(),
            );
        }
    }

    ExplainSummary {
        verbosity: verbosity.as_str(),
        stages,
        index_used,
        collscan,
        keys_examined,
        docs_examined,
        returned,
        execution_time_ms,
        advice,
    }
}

/// Depth-first search for the first object value under `key`.
fn find_key<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => {
            if let Some(found) = m.get(key) {
                return Some(found);
            }
            m.values().find_map(|child| find_key(child, key))
        }
        Value::Array(a) => a.iter().find_map(|child| find_key(child, key)),
        _ => None,
    }
}

/// Stage names, parent before child, following `inputStage`, `inputStages`, `queryPlan`, ...
fn collect_stages(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            if let Some(Value::String(s)) = m.get("stage") {
                out.push(s.clone());
            }
            for (k, child) in m {
                if k != "stage" {
                    collect_stages(child, out);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|c| collect_stages(c, out)),
        _ => {}
    }
}

/// Remove cluster-identifying and noisy fields before the explain document is returned.
pub fn scrub(v: &mut Value) {
    match v {
        Value::Object(m) => {
            for k in [
                "serverInfo",
                "serverParameters",
                "$clusterTime",
                "operationTime",
                "$configTime",
            ] {
                m.remove(k);
            }
            m.values_mut().for_each(scrub);
        }
        Value::Array(a) => a.iter_mut().for_each(scrub),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn collscan_on_tiny_data_gets_both_notes() {
        let raw = json!({
            "queryPlanner": { "winningPlan": { "stage": "COLLSCAN", "direction": "forward" } },
            "executionStats": { "nReturned": 1, "executionTimeMillis": 0, "totalKeysExamined": 0, "totalDocsExamined": 4 }
        });
        let s = summarize(&raw, Verbosity::ExecutionStats);
        assert!(s.collscan);
        assert_eq!(s.index_used, None);
        assert_eq!((s.docs_examined, s.returned), (Some(4), Some(1)));
        assert!(s.advice.iter().any(|a| a.starts_with("COLLSCAN")));
        assert!(s.advice.iter().any(|a| a.contains("tiny")));
    }

    #[test]
    fn index_scan_is_recognised() {
        let raw = json!({
            "queryPlanner": { "winningPlan": { "stage": "FETCH", "inputStage": { "stage": "IXSCAN", "indexName": "age_1" } } },
            "executionStats": { "nReturned": 3, "totalKeysExamined": 3, "totalDocsExamined": 3, "executionTimeMillis": 1 }
        });
        let s = summarize(&raw, Verbosity::ExecutionStats);
        assert_eq!(s.stages, ["FETCH", "IXSCAN"]);
        assert_eq!(s.index_used.as_deref(), Some("age_1"));
        assert!(!s.collscan);
        assert!(s.advice.iter().any(|a| a.contains("tight")));
    }

    #[test]
    fn aggregate_cursor_stage_is_found() {
        let raw = json!({ "stages": [ { "$cursor": {
            "queryPlanner": { "winningPlan": { "stage": "COLLSCAN" } },
            "executionStats": { "nReturned": 2, "totalDocsExamined": 2, "totalKeysExamined": 0, "executionTimeMillis": 0 }
        } }, { "$group": {} } ] });
        let s = summarize(&raw, Verbosity::ExecutionStats);
        assert!(s.collscan);
        assert_eq!(s.returned, Some(2));
    }

    #[test]
    fn query_planner_only_says_so() {
        let raw = json!({ "queryPlanner": { "winningPlan": { "stage": "COLLSCAN" } } });
        let s = summarize(&raw, Verbosity::QueryPlanner);
        assert!(s.advice.iter().any(|a| a.contains("executionStats")));
        assert_eq!(s.docs_examined, None);
    }

    #[test]
    fn wasteful_scan_is_flagged() {
        let raw = json!({
            "queryPlanner": { "winningPlan": { "stage": "COLLSCAN" } },
            "executionStats": { "nReturned": 1, "totalDocsExamined": 200, "totalKeysExamined": 0, "executionTimeMillis": 2 }
        });
        let s = summarize(&raw, Verbosity::ExecutionStats);
        assert!(s.advice.iter().any(|a| a.contains("much larger")));
    }

    #[test]
    fn atlas_search_pipeline_explains_are_summarised() {
        let raw = json!({
            "stages": [
                { "$_internalSearchMongotRemote": { "explain": { "query": { "type": "BooleanQuery" } } }, "nReturned": 2, "executionTimeMillisEstimate": 0 },
                { "$_internalSearchIdLookup": { "totalDocsExamined": 2, "totalKeysExamined": 2 }, "nReturned": 2, "executionTimeMillisEstimate": 1 },
                { "$project": { "title": true }, "nReturned": 2, "executionTimeMillisEstimate": 1 }
            ]
        });
        let s = summarize(&raw, Verbosity::ExecutionStats);
        assert_eq!(
            s.stages,
            [
                "$_internalSearchMongotRemote",
                "$_internalSearchIdLookup",
                "$project"
            ]
        );
        assert_eq!(
            (s.docs_examined, s.keys_examined, s.returned),
            (Some(2), Some(2), Some(2))
        );
        assert!(!s.collscan);
        assert!(s
            .advice
            .iter()
            .any(|a| a.contains("Atlas Search") && a.contains("BooleanQuery")));
        assert!(!s
            .advice
            .iter()
            .any(|a| a.contains("Only the plan was requested")));
    }

    #[test]
    fn scrub_removes_host_details() {
        let mut v =
            json!({ "serverInfo": { "host": "h" }, "a": { "serverParameters": {}, "keep": 1 } });
        scrub(&mut v);
        assert_eq!(v, json!({ "a": { "keep": 1 } }));
    }
}
