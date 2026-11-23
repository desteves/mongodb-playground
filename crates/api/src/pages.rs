//! Markdown for agents: every page of the site has a Markdown twin, served from the same URL.
//!
//! The web app is a single-page application, so its HTML is an empty shell until JavaScript runs, which is
//! useless to an agent. Instead:
//!
//! - a request that prefers `text/markdown` (`Accept: text/markdown`) for a page gets the page as Markdown;
//! - `<page>.md` (for example `/about.md`, `/docs/api.md`, `/index.md` for the home page) always gets Markdown;
//! - HTML responses for those pages say `Vary: Accept` and point at the Markdown twin with a `Link` header.
//!
//! Markdown responses carry `Content-Type: text/markdown; charset=utf-8`, `Vary: Accept`, `x-markdown-tokens`
//! (an estimate: bytes / 4) and `content-signal` (override with `CONTENT_SIGNAL`). The text lives in
//! `src/pages/*.md` with `{{placeholders}}` for the live limits, and the "run it yourself" page is generated
//! from the same guide the API serves. Links to mongodb.com get the UTM parameters.

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use playground_core::{tag_mongodb_links, Limits};
use serde_json::{json, Value};

const INDEX: &str = include_str!("pages/index.md");
const QUICKSTART: &str = include_str!("pages/quickstart.md");
const API: &str = include_str!("pages/api.md");
const MCP: &str = include_str!("pages/mcp.md");
const ABOUT: &str = include_str!("pages/about.md");

/// The site's pages: (path, template). The "run it yourself" page has no template; it is generated.
const PAGES: [(&str, &str); 6] = [
    ("/", INDEX),
    ("/docs/quickstart", QUICKSTART),
    ("/docs/api", API),
    ("/docs/mcp", MCP),
    ("/docs/run-it-yourself", ""),
    ("/about", ABOUT),
];

const DEFAULT_SIGNAL: &str = "ai-train=yes, search=yes, ai-input=yes";

/// Everything needed to render the pages; built once at start-up, no database involved.
pub struct PageCtx {
    origin: String,
    vars: Vec<(&'static str, String)>,
    signal: String,
    guide: Value,
}

impl PageCtx {
    pub fn new(origin: &str, l: &Limits, cluster: &str, signal: Option<String>) -> Arc<Self> {
        let origin = origin.trim_end_matches('/').to_string();
        let vars = vec![
            ("origin", origin.clone()),
            ("maxCollections", l.max_collections.to_string()),
            ("maxDocs", l.max_docs_per_collection.to_string()),
            (
                "maxPayloadMb",
                (l.max_payload_bytes / (1024 * 1024)).to_string(),
            ),
            ("maxTimeSeconds", (l.max_time_ms / 1000).to_string()),
            ("maxIndexes", l.max_indexes_per_collection.to_string()),
            (
                "shareTtlDays",
                (playground_core::SHARE_TTL_SECS / 86_400).to_string(),
            ),
            ("cluster", cluster.to_string()),
        ];
        Arc::new(Self {
            guide: playground_core::guide_for(&origin),
            origin,
            vars,
            signal: signal.unwrap_or_else(|| DEFAULT_SIGNAL.to_string()),
        })
    }

    /// The Markdown for a page path (`/`, `/about`, ...), or `None` if the path is not a page.
    ///
    /// Same layout as Cloudflare's "Markdown for Agents": YAML front matter (`title`, `description`, and
    /// `image` when there is one), the page body, then the page's JSON-LD in one fenced `json` block at the end.
    pub fn render(&self, path: &str) -> Option<String> {
        let (_, template) = PAGES.iter().find(|(p, _)| *p == path)?;
        let mut text = if path == "/docs/run-it-yourself" {
            self.run_it_yourself()
        } else {
            (*template).to_string()
        };
        for (k, v) in &self.vars {
            text = text.replace(&format!("{{{{{k}}}}}"), v);
        }
        let ld = self.json_ld(path, &text);
        if !ld.is_empty() {
            text.push_str("\n```json\n");
            let blocks: Vec<String> = ld
                .iter()
                .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
                .collect();
            text.push_str(&blocks.join("\n"));
            text.push_str("\n```\n");
        }
        Some(tag_mongodb_links(&text))
    }

    /// The app shell filled with this page's head tags, structured data and text (see `prerender`).
    pub fn prerender(&self, page: &str, shell: &str) -> Option<String> {
        let md = self.render(page)?;
        let (title, description) = front_matter(&md);
        let body = md
            .strip_prefix("---\n")
            .and_then(|r| r.split_once("\n---"))
            .map_or(md.as_str(), |(_, b)| b);
        // render() appends the page's JSON-LD as a last fenced block; the HTML carries it as script tags instead.
        let body = body.rfind("\n```json\n").map_or(body, |i| &body[..i]);
        let ld: Vec<String> = self
            .json_ld(page, &md)
            .iter()
            .map(|v| serde_json::to_string(v).unwrap_or_default())
            .collect();
        let canonical = format!("{}{}", self.origin, if page == "/" { "/" } else { page });
        Some(crate::prerender::apply(
            shell,
            &crate::prerender::Page {
                title: &title,
                description: &description,
                canonical: &canonical,
                markdown_twin: &twin(&self.origin, page),
                body_html: &crate::prerender::md_to_html(body),
                json_ld: &ld,
            },
        ))
    }

    /// The structured data (schema.org JSON-LD) for a page.
    fn json_ld(&self, path: &str, md: &str) -> Vec<Value> {
        let (title, description) = front_matter(md);
        let url = format!("{}{}", self.origin, if path == "/" { "/" } else { path });
        let site = json!({ "@type": "WebSite", "name": "MongoDB Playground", "url": format!("{}/", self.origin) });
        let ctx = "https://schema.org";
        match path {
            "/" => {
                let faq: Vec<Value> = faq_pairs(md)
                    .into_iter()
                    .map(|(q, a)| json!({ "@type": "Question", "name": q, "acceptedAnswer": { "@type": "Answer", "text": a } }))
                    .collect();
                vec![
                    json!({
                        "@context": ctx, "@type": "WebApplication", "name": "MongoDB Playground", "url": url,
                        "applicationCategory": "DeveloperApplication", "operatingSystem": "Any (web browser)",
                        "description": description, "isAccessibleForFree": true,
                        "offers": { "@type": "Offer", "price": "0", "priceCurrency": "USD" },
                        "sameAs": ["https://github.com/desteves/mongodb-playground"],
                        "featureList": [
                            "mongosh-style shell with several statements per run",
                            "Documents, indexes, Atlas Search and Vector Search indexes in one data panel",
                            "Query plans with plain-language advice",
                            "Share links and export to a mongosh script",
                            "REST API and MCP server for AI agents"
                        ]
                    }),
                    json!({ "@context": ctx, "@type": "FAQPage", "mainEntity": faq }),
                ]
            }
            "/docs/quickstart" => {
                let steps: Vec<Value> = numbered_sections(md)
                    .into_iter()
                    .map(|(name, text)| json!({ "@type": "HowToStep", "name": name, "text": text }))
                    .collect();
                vec![json!({
                    "@context": ctx, "@type": "HowTo", "name": "Test a MongoDB query online in 60 seconds",
                    "description": description, "url": url, "totalTime": "PT1M", "step": steps
                })]
            }
            "/about" => vec![json!({
                "@context": ctx, "@type": "AboutPage", "name": title, "description": description, "url": url, "isPartOf": site
            })],
            _ => vec![json!({
                "@context": ctx, "@type": "TechArticle", "headline": title, "description": description,
                "url": url, "inLanguage": "en", "isPartOf": site
            })],
        }
    }

    /// "Run it yourself" from the same guide that `GET /api/v1/guide` serves.
    fn run_it_yourself(&self) -> String {
        let g = &self.guide;
        let s = |v: &Value| v.as_str().unwrap_or("").to_string();
        let mut out = String::from(
            "---\ntitle: Run MongoDB Playground Locally or on a Free Atlas Cluster\n\
             description: Run the MongoDB playground on your machine with the Atlas CLI, Docker or the desktop app, or point it at a free-forever or ephemeral MongoDB Atlas cluster.\n---\n\n",
        );
        out.push_str(&format!("# {}\n\n{}\n\n", s(&g["title"]), s(&g["summary"])));
        if let Some(note) = g["note"].as_str() {
            out.push_str(&format!("{note}\n\n"));
        }
        let opts: Vec<&Value> = g["options"].as_array().into_iter().flatten().collect();
        if let Some(choose) = g["choose"].as_array() {
            out.push_str("## Which one should I use?\n\n");
            for c in choose {
                let title = opts
                    .iter()
                    .find(|o| o["id"] == c["use"])
                    .map(|o| s(&o["title"]))
                    .unwrap_or_default();
                out.push_str(&format!("- {}: **{title}**\n", s(&c["want"])));
            }
            out.push('\n');
        }
        let mut group = String::new();
        for o in opts {
            let gname = s(&o["group"]);
            if !gname.is_empty() && gname != group {
                out.push_str(&format!("## {gname}\n\n"));
                group = gname;
            }
            out.push_str(&format!("### {}\n\n", s(&o["title"])));
            if let Some(sum) = o["summary"].as_str() {
                out.push_str(&format!("{sum}\n\n"));
            }
            for (i, step) in o["steps"].as_array().into_iter().flatten().enumerate() {
                match step["url"].as_str() {
                    Some(u) => out.push_str(&format!("{}. [{}]({u})\n", i + 1, s(&step["label"]))),
                    None => out.push_str(&format!("{}. {}\n", i + 1, s(&step["label"]))),
                }
                if let Some(c) = step["command"].as_str() {
                    out.push_str(&format!("   ```bash\n   {c}\n   ```\n"));
                }
                if let Some(n) = step["note"].as_str() {
                    out.push_str(&format!("   {n}\n"));
                }
            }
            out.push('\n');
        }
        out.push_str(&format!(
            "All pages are available as Markdown: see [llms.txt]({}/llms.txt).\n",
            self.origin
        ));
        out
    }
}

/// `(title, description)` from a page's YAML front matter.
fn front_matter(md: &str) -> (String, String) {
    let head = md
        .strip_prefix("---\n")
        .and_then(|r| r.split("\n---").next())
        .unwrap_or("");
    let get = |key: &str| {
        head.lines()
            .find_map(|l| l.strip_prefix(&format!("{key}: ")))
            .unwrap_or("")
            .to_string()
    };
    (get("title"), get("description"))
}

/// The question and answer pairs under "Frequently asked questions": a `**question?**` line, then the answer.
fn faq_pairs(md: &str) -> Vec<(String, String)> {
    let Some(start) = md.find("## Frequently asked questions") else {
        return vec![];
    };
    let section = &md[start..];
    let end = section[3..].find("\n## ").map_or(section.len(), |i| i + 3);
    let mut pairs = vec![];
    for block in section[..end].split("\n\n") {
        let mut lines = block.lines();
        if let Some(q) = lines
            .next()
            .and_then(|l| l.strip_prefix("**"))
            .and_then(|l| l.strip_suffix("**"))
        {
            let a: Vec<&str> = lines.collect();
            if !a.is_empty() {
                pairs.push((q.to_string(), a.join(" ").replace('`', "")));
            }
        }
    }
    pairs
}

/// `(heading, first paragraph)` for each `## 1. Heading` style section.
fn numbered_sections(md: &str) -> Vec<(String, String)> {
    let mut out = vec![];
    let mut lines = md.lines().peekable();
    while let Some(l) = lines.next() {
        let Some(h) = l.strip_prefix("## ") else {
            continue;
        };
        if !h.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            continue;
        }
        let name = h.split_once(". ").map_or(h, |(_, n)| n).to_string();
        let mut text = String::new();
        while let Some(n) = lines.peek() {
            if n.starts_with("## ") {
                break;
            }
            let n = lines.next().unwrap_or_default().trim();
            if !n.is_empty() && !n.starts_with("```") && text.is_empty() {
                text = n.replace('`', "");
            }
        }
        out.push((name, text));
    }
    out
}

/// True when the client prefers Markdown over HTML: `text/markdown` is listed with a quality above zero and at
/// least that of `text/html` (browsers do not list it, so they keep getting HTML; `*/*` alone means HTML).
pub fn wants_markdown(accept: Option<&str>) -> bool {
    let Some(accept) = accept else { return false };
    let (mut md, mut html): (Option<f32>, Option<f32>) = (None, None);
    for part in accept.split(',') {
        let mut it = part.split(';');
        let kind = it.next().unwrap_or("").trim().to_ascii_lowercase();
        let q = it
            .filter_map(|p| {
                p.trim()
                    .strip_prefix("q=")
                    .and_then(|v| v.trim().parse::<f32>().ok())
            })
            .next()
            .unwrap_or(1.0);
        match kind.as_str() {
            "text/markdown" | "text/x-markdown" => md = Some(md.unwrap_or(0.0).max(q)),
            "text/html" | "application/xhtml+xml" => html = Some(html.unwrap_or(0.0).max(q)),
            _ => {}
        }
    }
    match (md, html) {
        (Some(m), Some(h)) => m > 0.0 && m >= h,
        (Some(m), None) => m > 0.0,
        _ => false,
    }
}

/// Maps a request path to (page path, asked for the `.md` twin explicitly).
fn resolve(path: &str) -> Option<(&'static str, bool)> {
    let trimmed = if path.len() > 1 {
        path.trim_end_matches('/')
    } else {
        path
    };
    let (base, explicit) = match trimmed.strip_suffix(".md") {
        Some("/index") => ("/", true),
        Some(b) => (b, true),
        None => (trimmed, false),
    };
    PAGES
        .iter()
        .find(|(p, _)| *p == base)
        .map(|(p, _)| (*p, explicit))
}

/// The URL of a page's Markdown twin.
fn twin(origin: &str, page: &str) -> String {
    match page {
        "/" => format!("{origin}/index.md"),
        p => format!("{origin}{p}.md"),
    }
}

/// Axum middleware (state: the page context). Markdown for agents that ask for it, HTML otherwise.
pub async fn negotiate(State(ctx): State<Arc<PageCtx>>, req: Request, next: Next) -> Response {
    let head = req.method() == Method::HEAD;
    if !head && req.method() != Method::GET {
        return next.run(req).await;
    }
    let Some((page, explicit)) = resolve(req.uri().path()) else {
        return next.run(req).await;
    };
    let accept = req
        .headers()
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok());
    if explicit || wants_markdown(accept) {
        if let Some(body) = ctx.render(page) {
            let tokens = body.len() / 4;
            let mut res = if head {
                ().into_response()
            } else {
                body.into_response()
            };
            let h = res.headers_mut();
            h.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/markdown; charset=utf-8"),
            );
            h.insert(header::VARY, HeaderValue::from_static("Accept"));
            h.insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=300"),
            );
            if let Ok(v) = HeaderValue::from_str(&tokens.to_string()) {
                h.insert("x-markdown-tokens", v);
            }
            if let Ok(v) = HeaderValue::from_str(&ctx.signal) {
                h.insert("content-signal", v);
            }
            let canonical = format!(
                "<{}{}>; rel=\"canonical\"",
                ctx.origin,
                if page == "/" { "/" } else { page }
            );
            if let Ok(v) = HeaderValue::from_str(&canonical) {
                h.insert(header::LINK, v);
            }
            return res;
        }
    }
    // HTML: the representation depends on Accept, and the Markdown twin is advertised.
    let mut res = next.run(req).await;
    let is_html = res.status() == axum::http::StatusCode::OK
        && res
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/html"));
    if is_html && !head {
        let (mut parts, body) = res.into_parts();
        match axum::body::to_bytes(body, 1 << 20).await {
            Ok(bytes) => {
                let shell = String::from_utf8_lossy(&bytes).into_owned();
                let filled = ctx.prerender(page, &shell).unwrap_or(shell);
                parts.headers.remove(header::CONTENT_LENGTH);
                parts.headers.remove(header::ETAG);
                parts.headers.remove(header::LAST_MODIFIED);
                res = Response::from_parts(parts, axum::body::Body::from(filled));
            }
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }
    let h = res.headers_mut();
    h.append(header::VARY, HeaderValue::from_static("Accept"));
    if let Ok(v) = HeaderValue::from_str(&format!(
        "<{}>; rel=\"alternate\"; type=\"text/markdown\"",
        twin(&ctx.origin, page)
    )) {
        h.append(header::LINK, v);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request as HttpRequest, routing::get, Router};
    use tower::ServiceExt;

    fn ctx() -> Arc<PageCtx> {
        PageCtx::new("https://playground.test/", &Limits::default(), "M10", None)
    }

    fn app() -> Router {
        Router::new()
            .fallback(get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/html")],
                    "<html>app shell</html>",
                )
            }))
            .layer(axum::middleware::from_fn_with_state(ctx(), negotiate))
    }

    async fn call(path: &str, accept: Option<&str>) -> (axum::http::response::Parts, String) {
        let mut req = HttpRequest::builder().uri(path);
        if let Some(a) = accept {
            req = req.header(header::ACCEPT, a);
        }
        let res = app()
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let (parts, body) = res.into_parts();
        (
            parts,
            String::from_utf8(axum::body::to_bytes(body, 1 << 20).await.unwrap().to_vec()).unwrap(),
        )
    }

    #[test]
    fn every_page_fills_the_real_app_shell_with_its_own_text_and_structured_data() {
        let shell = include_str!("../../../web/index.html");
        let c = ctx();
        for (path, _) in PAGES {
            let html = c.prerender(path, shell).unwrap();
            assert!(html.contains("<h1>"), "{path}: no heading");
            assert!(
                html.contains("application/ld+json"),
                "{path}: no structured data"
            );
            assert_eq!(
                html.matches("application/ld+json").count(),
                if path == "/" { 2 } else { 1 }
            );
            assert!(
                html.contains(&format!(
                    "href=\"https://playground.test{}\"",
                    if path == "/" { "/" } else { path }
                )),
                "{path}: canonical"
            );
            assert!(!html.contains("{{"), "{path}: leftover placeholder");
        }
        let home = c.prerender("/", shell).unwrap();
        assert!(home.contains("FAQPage") && home.contains("Frequently asked questions"));
        let quick = c.prerender("/docs/quickstart", shell).unwrap();
        assert!(quick.contains("HowTo"));
    }

    #[test]
    fn markdown_is_chosen_only_when_asked_for() {
        assert!(wants_markdown(Some("text/markdown")));
        assert!(wants_markdown(Some("text/markdown, text/html;q=0.8")));
        assert!(wants_markdown(Some("text/html;q=0.5, text/markdown;q=0.9")));
        assert!(!wants_markdown(Some(
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
        )));
        assert!(!wants_markdown(Some("text/html, text/markdown;q=0.5")));
        assert!(!wants_markdown(Some("text/markdown;q=0")));
        assert!(!wants_markdown(Some("*/*")));
        assert!(!wants_markdown(Some("application/json")));
        assert!(!wants_markdown(None));
    }

    #[test]
    fn every_page_renders_with_front_matter_and_no_leftover_placeholders() {
        let c = ctx();
        for (path, _) in PAGES {
            let md = c
                .render(path)
                .unwrap_or_else(|| panic!("{path} did not render"));
            assert!(md.starts_with("---\ntitle: "), "{path}: front matter");
            assert!(md.contains("\n# "), "{path}: heading");
            assert!(!md.contains("{{"), "{path}: unreplaced placeholder");
        }
        assert!(c.render("/nope").is_none());
        let home = c.render("/").unwrap();
        assert!(
            home.contains("https://playground.test/docs/api.md")
                && !home.contains("playground.test//")
        );
    }

    /// Cloudflare's layout: YAML front matter (title, description, image only), the body, then every JSON-LD
    /// object in one fenced `json` block at the end.
    #[test]
    fn output_follows_the_markdown_for_agents_layout() {
        let c = ctx();
        for (path, _) in PAGES {
            let md = c.render(path).unwrap();
            let (front, rest) = md
                .strip_prefix("---\n")
                .unwrap()
                .split_once("\n---\n")
                .unwrap();
            for line in front.lines() {
                let key = line.split(':').next().unwrap();
                assert!(
                    ["title", "description", "image"].contains(&key),
                    "{path}: unexpected key {key}"
                );
            }
            assert!(
                front.contains("title: ") && front.contains("description: "),
                "{path}"
            );
            assert!(
                rest.trim_start().starts_with("# "),
                "{path}: body starts with the page heading"
            );
            assert!(
                md.trim_end().ends_with("```"),
                "{path}: ends with the JSON-LD block"
            );
            let json = md
                .rsplit("```json\n")
                .next()
                .unwrap()
                .trim_end()
                .trim_end_matches("```");
            let objects: Vec<Value> = serde_json::Deserializer::from_str(json)
                .into_iter::<Value>()
                .collect::<Result<_, _>>()
                .unwrap_or_else(|e| panic!("{path}: JSON-LD does not parse: {e}"));
            assert!(!objects.is_empty());
            for o in &objects {
                assert_eq!(o["@context"], "https://schema.org", "{path}");
                assert!(o["@type"].is_string(), "{path}");
            }
        }
        let home = c.render("/").unwrap();
        let faq: Vec<_> = faq_pairs(&home);
        assert!(
            faq.len() >= 6 && faq.iter().all(|(q, a)| q.ends_with('?') && !a.is_empty()),
            "{faq:?}"
        );
        assert!(
            home.contains("\"@type\": \"FAQPage\"")
                && home.contains("\"@type\": \"WebApplication\"")
        );
        let quick = c.render("/docs/quickstart").unwrap();
        assert!(
            quick.contains("\"@type\": \"HowTo\"") && quick.contains("\"@type\": \"HowToStep\"")
        );
        assert_eq!(numbered_sections(&quick).len(), 7);
    }

    #[test]
    fn run_it_yourself_is_grouped_and_ephemeral_clusters_are_described_correctly() {
        let md = ctx().render("/docs/run-it-yourself").unwrap();
        for h in [
            "## Which one should I use?",
            "## Use it now",
            "## Run it on your machine",
            "## Use a real Atlas cluster",
        ] {
            assert!(md.contains(h), "{h}");
        }
        let eph = md.split("### Ephemeral Atlas cluster").nth(1).unwrap();
        assert!(
            eph.contains("7 days")
                && eph.contains("atlas api ephemeralClusters createEphemeralCluster")
        );
        assert!(!md.contains("create, use, delete"));
    }

    #[test]
    fn links_to_mongodb_com_carry_the_utm_parameters() {
        let md = ctx().render("/docs/quickstart").unwrap();
        assert!(md.contains("https://www.mongodb.com/cloud/atlas/register?utm_campaign=devrel&utm_source=third-party-content"));
        assert_eq!(
            tag_mongodb_links("see (https://github.com/x) and https://example.com"),
            "see (https://github.com/x) and https://example.com"
        );
        assert!(
            tag_mongodb_links("<https://www.mongodb.com/docs/atlas/cli/>")
                .contains("/docs/atlas/cli/?utm_campaign=devrel")
        );
    }

    #[test]
    fn paths_resolve_with_and_without_md_and_slashes() {
        assert_eq!(resolve("/"), Some(("/", false)));
        assert_eq!(resolve("/index.md"), Some(("/", true)));
        assert_eq!(resolve("/about.md"), Some(("/about", true)));
        assert_eq!(resolve("/docs/api/"), Some(("/docs/api", false)));
        assert_eq!(resolve("/docs/api.md"), Some(("/docs/api", true)));
        assert_eq!(resolve("/p/some-id"), None);
        assert_eq!(resolve("/assets/index.js"), None);
    }

    #[tokio::test]
    async fn accept_header_and_md_suffix_serve_markdown_and_browsers_get_html() {
        let (p, body) = call("/about", Some("text/markdown")).await;
        assert_eq!(
            p.headers[header::CONTENT_TYPE],
            "text/markdown; charset=utf-8"
        );
        assert_eq!(p.headers[header::VARY], "Accept");
        assert!(
            p.headers.contains_key("x-markdown-tokens") && p.headers.contains_key("content-signal")
        );
        assert!(body.contains("# About MongoDB Playground"));

        let (p, body) = call("/docs/mcp.md", None).await;
        assert_eq!(
            p.headers[header::CONTENT_TYPE],
            "text/markdown; charset=utf-8"
        );
        assert!(body.contains("run_query"));

        let (p, body) = call("/", Some("text/html,application/xhtml+xml,*/*;q=0.8")).await;
        assert_eq!(p.headers[header::CONTENT_TYPE], "text/html");
        assert_eq!(body, "<html>app shell</html>");
        assert!(p.headers[header::VARY].to_str().unwrap().contains("Accept"));
        assert!(p.headers[header::LINK]
            .to_str()
            .unwrap()
            .contains("https://playground.test/index.md"));

        // not a page: untouched
        let (p, _) = call("/p/abc", Some("text/markdown")).await;
        assert_eq!(p.headers[header::CONTENT_TYPE], "text/html");
    }
}
