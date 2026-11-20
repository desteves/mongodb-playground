//! Core logic for the MongoDB playground: request types, validation, and the runner.
//!
//! Nothing here depends on an HTTP framework, so the API service and the Tauri desktop
//! shell can both use it.

mod error;
mod explain;
mod export;
mod limits;
mod relaxed;
mod request;
mod runner;
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
pub use share::{is_valid_id, Loaded, SharedPlayground, Shares, SHARE_TTL_SECS};
pub use shell::{prepare_script, Op, PreparedScript, Statement, MAX_STATEMENTS};
pub use stats::Stats;
pub use validate::{prepare, prepare_ok_summary, Prepared, Query};

/// Ready-to-run example requests (each is a valid body for `POST /api/v1/run`).
pub fn examples() -> serde_json::Value {
    serde_json::from_str(include_str!("examples.json")).expect("examples.json is valid JSON")
}

/// How to run the playground yourself, get a free or ephemeral Atlas cluster, or connect an agent.
/// Text contains `{origin}` (the public URL of this server) and `{atlasRegister}` (the Atlas
/// sign-up link with UTM parameters); the API fills both in. Served at `GET /api/v1/guide`.
pub fn guide() -> serde_json::Value {
    serde_json::from_str(include_str!("guide.json")).expect("guide.json is valid JSON")
}

/// The guide with `{origin}` and `{atlasRegister}` filled in for a server at `origin`.
pub fn guide_for(origin: &str) -> serde_json::Value {
    let text = guide()
        .to_string()
        .replace("{origin}", origin.trim_end_matches('/'))
        .replace("{atlasRegister}", &atlas_register_url("guide"));
    serde_json::from_str(&text).expect("the filled-in guide is valid JSON")
}

/// The Atlas registration URL with UTM parameters. `content` says where the link was shown.
pub fn atlas_register_url(content: &str) -> String {
    let g = guide();
    let base = g["atlasRegister"]["url"]
        .as_str()
        .unwrap_or("https://www.mongodb.com/cloud/atlas/register");
    let mut url = base.to_string();
    let mut sep = '?';
    if let Some(utm) = g["atlasRegister"]["utm"].as_object() {
        for (k, v) in utm {
            url.push(sep);
            url.push_str(&format!("{k}={}", v.as_str().unwrap_or("")));
            sep = '&';
        }
    }
    url.push(sep);
    url.push_str(&format!("utm_content={content}"));
    url
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
                "atlas-free",
                "atlas-ephemeral",
                "docker",
                "desktop"
            ]
        );
        for o in g["options"].as_array().unwrap() {
            assert!(!o["steps"].as_array().unwrap().is_empty(), "{o}");
        }
        assert!(g.to_string().contains("{origin}"));
    }

    #[test]
    fn atlas_link_has_proper_utms() {
        let url = atlas_register_url("header");
        assert!(
            url.starts_with("https://www.mongodb.com/cloud/atlas/register?"),
            "{url}"
        );
        for part in [
            "utm_source=mongoplayground",
            "utm_medium=referral",
            "utm_campaign=playground-free-cluster",
            "utm_content=header",
        ] {
            assert!(url.contains(part), "{url} lacks {part}");
        }
    }
}
