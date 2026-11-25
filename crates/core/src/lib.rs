//! Core logic for the MongoDB playground: request types, validation, and the runner.
//!
//! Nothing here depends on an HTTP framework, so the API service and the Tauri desktop
//! shell can both use it.

mod error;
mod explain;
mod export;
pub mod format;
mod limits;
mod relaxed;
mod request;
mod runner;
pub mod seed;
mod share;
mod shell;
mod shellvalue;
pub mod stats;
mod validate;

pub use error::PlaygroundError;
pub use explain::{summarize, ExplainOutput, ExplainSummary};
pub use export::{export, shell_value, Exported};
pub use limits::Limits;
pub use relaxed::parse as parse_relaxed;
pub use request::{
    setup_from_value, CollectionSetup, Datasource, ExportRequest, IndexSpec, Mode, RunBody,
    RunRequest, ScriptRequest, SearchIndexSpec, Setup, Verbosity,
};
pub use runner::{
    make_client, ErrorBody, Reply, RunOutput, Runner, ScriptOutput, StatementOutput, WriteStats,
};
pub use share::{is_valid_id, Loaded, SharedPlayground, Shares, Usage, SHARE_TTL_SECS};
pub use shell::{prepare_script, Op, PreparedScript, Statement, MAX_STATEMENTS};
pub use stats::Stats;
pub use validate::{prepare, prepare_ok_summary, Prepared, Query};

/// Ready-to-run example requests (each is a valid body for `POST /api/v1/run`).
pub fn examples() -> serde_json::Value {
    serde_json::from_str(include_str!("examples.json")).expect("examples.json is valid JSON")
}

/// The UTM parameters on every link back to mongodb.com (any subdomain), in this order. One place only:
/// the Rust helpers, `GET /api/v1/guide` (`atlasRegister.utm`) and the web app all use this set.
pub const UTM: [(&str, &str); 5] = [
    ("utm_campaign", "devrel"),
    ("utm_source", "third-party-content"),
    ("utm_medium", "cta"),
    ("utm_content", "mongodb-playground"),
    ("utm_term", "diana.esteves"),
];

/// Adds the UTM parameters to a link to mongodb.com (`mongodb.com` or any `*.mongodb.com`: www, cloud,
/// docs, dochub, ...). Other links are returned unchanged; existing `utm_*` values from the set are replaced
/// and other query parameters and the fragment are kept.
pub fn with_utm(url: &str) -> String {
    let (no_fragment, fragment) = match url.split_once('#') {
        Some((a, b)) => (a, Some(b)),
        None => (url, None),
    };
    let (base, query) = no_fragment.split_once('?').unwrap_or((no_fragment, ""));
    let host = base
        .split_once("://")
        .map_or(base, |(_, rest)| rest)
        .split('/')
        .next()
        .unwrap_or("")
        .rsplit('@')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if host != "mongodb.com" && !host.ends_with(".mongodb.com") {
        return url.to_string();
    }
    let mut parts: Vec<String> = query
        .split('&')
        .filter(|p| !p.is_empty() && !UTM.iter().any(|(k, _)| p.split('=').next() == Some(*k)))
        .map(String::from)
        .collect();
    parts.extend(UTM.iter().map(|(k, v)| format!("{k}={v}")));
    let mut out = format!("{base}?{}", parts.join("&"));
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// Adds the UTM parameters to every link to mongodb.com in a piece of text (other links are left alone).
pub fn tag_mongodb_links(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 64);
    let mut rest = text;
    while let Some(i) = rest.find("https://") {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, ')' | '>' | '"' | '`' | '\'' | ']'))
            .unwrap_or(tail.len());
        out.push_str(&with_utm(&tail[..end]));
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// How to run the playground yourself, get a free or ephemeral Atlas cluster, or connect an agent.
/// Text contains `{origin}` (the public URL of this server) and `{atlasRegister}` (the Atlas
/// sign-up link with UTM parameters); the API fills both in. Served at `GET /api/v1/guide`.
pub fn guide() -> serde_json::Value {
    let mut g: serde_json::Value =
        serde_json::from_str(include_str!("guide.json")).expect("guide.json is valid JSON");
    let utm: serde_json::Map<String, serde_json::Value> = UTM
        .iter()
        .map(|(k, v)| {
            (
                (*k).to_string(),
                serde_json::Value::String((*v).to_string()),
            )
        })
        .collect();
    g["atlasRegister"]["utm"] = serde_json::Value::Object(utm);
    g
}

/// The guide with `{origin}` and `{atlasRegister}` filled in for a server at `origin`.
pub fn guide_for(origin: &str) -> serde_json::Value {
    let text = guide()
        .to_string()
        .replace("{origin}", origin.trim_end_matches('/'))
        .replace("{atlasRegister}", &atlas_register_url());
    // Every link back to mongodb.com carries the UTM parameters.
    serde_json::from_str(&tag_mongodb_links(&text)).expect("the filled-in guide is valid JSON")
}

/// The Atlas registration link, with the UTM parameters.
pub fn atlas_register_url() -> String {
    let g = guide();
    let base = g["atlasRegister"]["url"]
        .as_str()
        .unwrap_or("https://www.mongodb.com/cloud/atlas/register");
    with_utm(base)
}

#[cfg(test)]
mod guide_tests {
    use super::*;

    #[test]
    fn guide_lists_every_way_to_run_it_with_commands() {
        let g = guide();
        let ids: Vec<_> = g["options"]
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "hosted",
                "mcp",
                "atlas-local",
                "docker",
                "desktop",
                "atlas-free",
                "atlas-ephemeral"
            ]
        );
        for o in g["options"].as_array().unwrap() {
            assert!(!o["steps"].as_array().unwrap().is_empty(), "{o}");
            assert!(o["group"].as_str().is_some_and(|s| !s.is_empty()), "{o}");
        }
        // the chooser points at real options
        for c in g["choose"].as_array().unwrap() {
            assert!(ids.contains(&c["use"].as_str().unwrap()), "{c}");
        }
        // an ephemeral Atlas cluster is temporary: the guide must not describe it as create-use-delete
        let eph = g["options"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["id"] == "atlas-ephemeral")
            .unwrap()
            .to_string();
        assert!(eph.contains("7 days") && eph.contains("ephemeralClusters createEphemeralCluster"));
        assert!(g.to_string().contains("{origin}"));
    }

    const EXPECTED: &str = "utm_campaign=devrel&utm_source=third-party-content&utm_medium=cta&utm_content=mongodb-playground&utm_term=diana.esteves";

    #[test]
    fn atlas_link_has_proper_utms() {
        assert_eq!(
            atlas_register_url(),
            format!("https://www.mongodb.com/cloud/atlas/register?{EXPECTED}")
        );
    }

    #[test]
    fn every_mongodb_com_link_gets_the_utms_and_nothing_else_does() {
        for url in [
            "https://mongodb.com/docs/",
            "https://www.mongodb.com/docs/atlas/cli/",
            "https://cloud.mongodb.com/v2",
            "https://dochub.mongodb.com/core/x",
            "HTTPS://WWW.MONGODB.COM/docs",
        ] {
            assert!(with_utm(url).contains(EXPECTED), "{url}");
        }
        // existing parameters and the fragment are kept; old utm values are replaced
        assert_eq!(
            with_utm("https://www.mongodb.com/docs?a=1&utm_source=old#top"),
            format!("https://www.mongodb.com/docs?a=1&{EXPECTED}#top")
        );
        // other hosts are untouched, including look-alikes and the connection-string host
        for url in [
            "https://github.com/desteves/mongodb-playground",
            "https://notmongodb.com/",
            "https://mongodb.com.evil.example/",
            "mongodb+srv://cluster0.abcde.mongodb.net/",
            "https://cloud.mongodb.net/",
        ] {
            assert_eq!(with_utm(url), url);
        }
    }

    #[test]
    fn the_guide_carries_the_same_set() {
        let g = guide();
        for (k, v) in UTM {
            assert_eq!(g["atlasRegister"]["utm"][k], v);
        }
    }
}
