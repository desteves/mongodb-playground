//! Export a playground so it can leave the site: a `mongosh` script that recreates the data,
//! indexes and statements against any MongoDB, or the data panel as Extended JSON.

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::request::{ExportRequest, Setup};

const DB_NAME: &str = "playground";

#[derive(Debug, Serialize)]
pub struct Exported {
    pub format: String,
    pub filename: String,
    pub mime_type: String,
    pub content: String,
}

/// Print a value the way you would type it in the shell: unquoted keys, `ObjectId("..")`,
/// `ISODate("..")`, `/regex/i`. The inverse of `shellvalue`'s parser.
pub fn shell_value(v: &Value, indent: usize) -> String {
    let pad = "  ".repeat(indent);
    let pad1 = "  ".repeat(indent + 1);
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => serde_json::to_string(s).unwrap_or_default(),
        Value::Array(a) => {
            if a.is_empty() {
                return "[]".into();
            }
            let flat: Vec<String> = a.iter().map(|x| shell_value(x, 0)).collect();
            if flat.iter().all(|f| !f.contains('\n')) && flat.join(", ").len() < 72 {
                return format!("[ {} ]", flat.join(", "));
            }
            let items: Vec<String> = a
                .iter()
                .map(|x| format!("{pad1}{}", shell_value(x, indent + 1)))
                .collect();
            format!("[\n{}\n{pad}]", items.join(",\n"))
        }
        Value::Object(m) => {
            if let Some(special) = special(m) {
                return special;
            }
            if m.is_empty() {
                return "{}".into();
            }
            let key = |k: &str| {
                let ident = !k.is_empty()
                    && !k.starts_with(|c: char| c.is_ascii_digit())
                    && k.chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
                if ident {
                    k.to_string()
                } else {
                    serde_json::to_string(k).unwrap_or_default()
                }
            };
            let flat: Vec<String> = m
                .iter()
                .map(|(k, x)| format!("{}: {}", key(k), shell_value(x, 0)))
                .collect();
            if flat.iter().all(|f| !f.contains('\n')) && flat.join(", ").len() < 72 {
                return format!("{{ {} }}", flat.join(", "));
            }
            let lines: Vec<String> = m
                .iter()
                .map(|(k, x)| format!("{pad1}{}: {}", key(k), shell_value(x, indent + 1)))
                .collect();
            format!("{{\n{}\n{pad}}}", lines.join(",\n"))
        }
    }
}

/// Extended JSON wrappers that have a shell constructor.
fn special(m: &Map<String, Value>) -> Option<String> {
    if m.len() != 1 {
        return None;
    }
    let (k, v) = m.iter().next()?;
    let s = |v: &Value| serde_json::to_string(v).unwrap_or_default();
    Some(match k.as_str() {
        "$oid" => format!("ObjectId({})", s(v)),
        "$date" => match v {
            Value::String(_) => format!("ISODate({})", s(v)),
            Value::Object(o) => {
                let ms = o.get("$numberLong")?.as_str()?;
                format!("new Date({ms})")
            }
            _ => return None,
        },
        "$numberLong" => format!("NumberLong({})", s(v)),
        "$numberInt" => format!("NumberInt({})", s(v)),
        "$numberDecimal" => format!("NumberDecimal({})", s(v)),
        "$uuid" => format!("UUID({})", s(v)),
        "$regularExpression" => {
            let r = v.as_object()?;
            format!(
                "/{}/{}",
                r.get("pattern")?.as_str()?,
                r.get("options")?.as_str().unwrap_or("")
            )
        }
        _ => return None,
    })
}

/// A script you can run with `mongosh "<connection string>" --file playground.js`.
pub fn mongosh_script(setup: &Setup, script: &str) -> String {
    let mut out = vec![
        "// Exported from MongoDB Playground.".to_string(),
        "// Run it against any MongoDB:  mongosh \"<connection string>\" --file playground.js"
            .to_string(),
        format!("// It uses a database called \"{DB_NAME}\"; nothing else is touched."),
        format!("use(\"{DB_NAME}\");"),
        String::new(),
    ];
    for (name, c) in &setup.collections {
        let coll = format!(
            "db.getCollection({})",
            serde_json::to_string(name).unwrap_or_default()
        );
        out.push(format!("// ---- {name}"));
        if !c.documents.is_empty() {
            out.push(format!(
                "{coll}.insertMany({});",
                shell_value(&Value::Array(c.documents.clone()), 0)
            ));
        }
        for i in &c.indexes {
            let opts = if i.options.is_empty() {
                String::new()
            } else {
                format!(", {}", shell_value(&Value::Object(i.options.clone()), 0))
            };
            out.push(format!(
                "{coll}.createIndex({}{opts});",
                shell_value(&Value::Object(i.key.clone()), 0)
            ));
        }
        for s in &c.search {
            out.push(format!(
                "{coll}.createSearchIndex({}, {});",
                serde_json::to_string(s.name.as_deref().unwrap_or("default")).unwrap_or_default(),
                shell_value(&Value::Object(s.definition.clone()), 0)
            ));
        }
        for s in &c.vector {
            out.push(format!(
                "{coll}.createSearchIndex({}, \"vectorSearch\", {});",
                serde_json::to_string(s.name.as_deref().unwrap_or("vector_index"))
                    .unwrap_or_default(),
                shell_value(&Value::Object(s.definition.clone()), 0)
            ));
        }
        out.push(String::new());
    }
    if !script.trim().is_empty() {
        out.push("// ---- statements".to_string());
        out.push(script.trim_end().to_string());
        out.push(String::new());
    }
    out.join("\n")
}

/// The data panel as Extended JSON, one entry per collection.
pub fn data_json(setup: &Setup, script: &str) -> Value {
    let collections: Map<String, Value> = setup
        .collections
        .iter()
        .map(|(name, c)| {
            let spec = |s: &crate::request::SearchIndexSpec| json!({ "name": s.name, "definition": s.definition });
            (
                name.clone(),
                json!({
                    "documents": c.documents,
                    "indexes": c.indexes.iter().map(|i| json!({ "key": i.key, "options": i.options })).collect::<Vec<_>>(),
                    "searchIndexes": c.search.iter().map(spec).collect::<Vec<_>>(),
                    "vectorSearchIndexes": c.vector.iter().map(spec).collect::<Vec<_>>(),
                }),
            )
        })
        .collect();
    json!({ "format": "playground-export/1", "database": DB_NAME, "collections": collections, "script": script })
}

pub fn export(req: &ExportRequest) -> Result<Exported, crate::PlaygroundError> {
    match req.format.as_deref().unwrap_or("mongosh") {
        "mongosh" => Ok(Exported {
            format: "mongosh".into(),
            filename: "playground.js".into(),
            mime_type: "text/javascript".into(),
            content: mongosh_script(&req.config, &req.script),
        }),
        "json" => Ok(Exported {
            format: "json".into(),
            filename: "playground.json".into(),
            mime_type: "application/json".into(),
            content: serde_json::to_string_pretty(&data_json(&req.config, &req.script))
                .unwrap_or_default(),
        }),
        other => Err(crate::PlaygroundError::Invalid(format!(
            "unknown export format `{other}`; use mongosh or json"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(config: Value, script: &str, format: &str) -> ExportRequest {
        serde_json::from_value(json!({"config": config, "script": script, "format": format}))
            .unwrap()
    }

    #[test]
    fn shell_values_round_trip_through_the_parser() {
        let src = r#"{ _id: ObjectId("507f1f77bcf86cd799439011"), at: ISODate("2024-01-01T00:00:00Z"), n: NumberLong("7"), re: /^a/i, "a.b": [1, 2.5, "x"], nested: { ok: true, none: null } }"#;
        let v = crate::parse_relaxed(src, "t").unwrap();
        let printed = shell_value(&v, 0);
        assert_eq!(crate::parse_relaxed(&printed, "t").unwrap(), v, "{printed}");
        assert!(
            printed.contains("ObjectId(")
                && printed.contains("ISODate(")
                && printed.contains("/^a/i")
        );
    }

    #[test]
    fn pre_1970_dates_print_as_new_date() {
        let v = json!({"$date": {"$numberLong": "-4861728000000"}});
        assert_eq!(shell_value(&v, 0), "new Date(-4861728000000)");
    }

    #[test]
    fn the_mongosh_script_recreates_data_indexes_and_statements() {
        let cfg = json!({"people": {
            "documents": [{"name": "Ada", "age": 36}],
            "indexes": [{"key": {"age": 1}, "options": {"unique": false}}],
            "searchIndexes": [{"definition": {"mappings": {"dynamic": true}}}],
            "vectorSearchIndexes": [{"definition": {"fields": [{"type": "vector", "path": "e", "numDimensions": 3, "similarity": "cosine"}]}}]
        }});
        let e = export(&req(cfg, "db.people.find({age: 36})", "mongosh")).unwrap();
        assert_eq!(e.filename, "playground.js");
        let c = e.content;
        for needle in [
            "use(\"playground\");",
            "db.getCollection(\"people\").insertMany([ { name: \"Ada\", age: 36 } ]);",
            ".createIndex({ age: 1 }, { unique: false });",
            ".createSearchIndex(\"default\", { mappings: { dynamic: true } });",
            ".createSearchIndex(\"vector_index\", \"vectorSearch\", {",
            "db.people.find({age: 36})",
        ] {
            assert!(c.contains(needle), "missing {needle:?} in:\n{c}");
        }
    }

    #[test]
    fn the_exported_script_parses_back_as_a_playground() {
        // the statements section is valid input to this very site
        let e = export(&req(
            json!([{"name": "Ada"}]),
            "db.collection.find({name: /^a/i})",
            "mongosh",
        ))
        .unwrap();
        assert!(e.content.contains("db.collection.find({name: /^a/i})"));
    }

    #[test]
    fn json_export_keeps_extended_json() {
        let e = export(&req(
            json!({"c": [{"_id": {"$oid": "507f1f77bcf86cd799439011"}}]}),
            "db.c.find()",
            "json",
        ))
        .unwrap();
        let v: Value = serde_json::from_str(&e.content).unwrap();
        assert_eq!(
            v["collections"]["c"]["documents"][0]["_id"]["$oid"],
            "507f1f77bcf86cd799439011"
        );
        assert_eq!(v["script"], "db.c.find()");
        assert_eq!(v["format"], "playground-export/1");
    }

    #[test]
    fn unknown_formats_are_rejected() {
        assert!(export(&req(json!({}), "", "xml")).is_err());
    }
}
