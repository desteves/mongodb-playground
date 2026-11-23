//! Server-side text for the HTML pages. The web app is a single-page application, so its HTML starts as
//! an empty shell; crawlers that do not run JavaScript (most AI crawlers) would see nothing. For each page
//! the server fills the shell with that page's title, description, canonical URL, structured data and the
//! page text (the same Markdown the `.md` twin serves, converted to plain HTML). The app replaces it on load.

/// Escape text for HTML.
pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// `code`, **bold** and [links](url) inside one line of Markdown. Only http(s) and site-relative links stay links.
fn inline(line: &str) -> String {
    let mut out = String::new();
    for (i, part) in line.split('`').enumerate() {
        if i % 2 == 1 {
            out.push_str(&format!("<code>{}</code>", esc(part)));
        } else {
            out.push_str(&emphasis_and_links(part));
        }
    }
    out
}

fn emphasis_and_links(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(open) = rest.find('[') {
        let (before, after) = rest.split_at(open);
        let link = after.find("](").and_then(|mid| {
            let end = after[mid + 2..].find(')')?;
            Some((
                &after[1..mid],
                &after[mid + 2..mid + 2 + end],
                mid + 2 + end + 1,
            ))
        });
        match link {
            Some((text, url, used)) if url.starts_with("http") || url.starts_with('/') => {
                out.push_str(&bold(before));
                out.push_str(&format!("<a href=\"{}\">{}</a>", esc(url), bold(text)));
                rest = &after[used..];
            }
            _ => {
                out.push_str(&bold(before));
                out.push('[');
                rest = &after[1..];
            }
        }
    }
    out.push_str(&bold(rest));
    out
}

fn bold(s: &str) -> String {
    s.split("**")
        .enumerate()
        .map(|(i, p)| {
            if i % 2 == 1 {
                format!("<strong>{}</strong>", esc(p))
            } else {
                esc(p)
            }
        })
        .collect()
}

/// The body of a page (without front matter) as simple HTML: headings, paragraphs, lists, code blocks.
pub fn md_to_html(md: &str) -> String {
    let mut out = String::new();
    let mut para: Vec<String> = vec![];
    let mut list: Option<&'static str> = None;
    let mut code: Option<String> = None;

    fn flush_para(out: &mut String, para: &mut Vec<String>) {
        if !para.is_empty() {
            out.push_str(&format!("<p>{}</p>\n", para.join(" ")));
            para.clear();
        }
    }
    fn close_list(out: &mut String, list: &mut Option<&'static str>) {
        if let Some(tag) = list.take() {
            out.push_str(&format!("</{tag}>\n"));
        }
    }

    for line in md.lines() {
        if let Some(block) = code.as_mut() {
            if line.trim_start().starts_with("```") {
                out.push_str(&format!(
                    "<pre><code>{}</code></pre>\n",
                    esc(block.trim_end())
                ));
                code = None;
            } else {
                block.push_str(line);
                block.push('\n');
            }
            continue;
        }
        let t = line.trim();
        if t.starts_with("```") {
            flush_para(&mut out, &mut para);
            close_list(&mut out, &mut list);
            code = Some(String::new());
        } else if t.is_empty() {
            flush_para(&mut out, &mut para);
            close_list(&mut out, &mut list);
        } else if let Some((level, text)) = heading(t) {
            flush_para(&mut out, &mut para);
            close_list(&mut out, &mut list);
            out.push_str(&format!("<h{level}>{}</h{level}>\n", inline(text)));
        } else if let Some(item) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            flush_para(&mut out, &mut para);
            if list != Some("ul") {
                close_list(&mut out, &mut list);
                out.push_str("<ul>\n");
                list = Some("ul");
            }
            out.push_str(&format!("<li>{}</li>\n", inline(item)));
        } else if let Some(item) = numbered(t) {
            flush_para(&mut out, &mut para);
            if list != Some("ol") {
                close_list(&mut out, &mut list);
                out.push_str("<ol>\n");
                list = Some("ol");
            }
            out.push_str(&format!("<li>{}</li>\n", inline(item)));
        } else if t.starts_with('|') {
            // Table rows: keep the text, drop the separator row.
            if !t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ')) {
                flush_para(&mut out, &mut para);
                out.push_str(&format!(
                    "<p>{}</p>\n",
                    inline(&t.trim_matches('|').replace('|', " · "))
                ));
            }
        } else {
            para.push(inline(t));
        }
    }
    flush_para(&mut out, &mut para);
    close_list(&mut out, &mut list);
    out
}

fn heading(t: &str) -> Option<(usize, &str)> {
    let hashes = t.chars().take_while(|c| *c == '#').count();
    (1..=4).contains(&hashes).then_some(())?;
    t[hashes..].strip_prefix(' ').map(|r| (hashes, r))
}

fn numbered(t: &str) -> Option<&str> {
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    (digits > 0).then_some(())?;
    t[digits..].strip_prefix(". ")
}

/// What one page contributes to the shell.
pub struct Page<'a> {
    pub title: &'a str,
    pub description: &'a str,
    pub canonical: &'a str,
    pub markdown_twin: &'a str,
    pub body_html: &'a str,
    /// Serialized JSON-LD blocks.
    pub json_ld: &'a [String],
}

/// Replace an attribute value that follows `prefix` (up to the next quote). Leaves the HTML alone when the
/// prefix is missing.
fn set_attr(html: &mut String, prefix: &str, value: &str) {
    if let Some(start) = html.find(prefix) {
        let from = start + prefix.len();
        if let Some(len) = html[from..].find('"') {
            html.replace_range(from..from + len, &esc(value));
        }
    }
}

/// Fill the app shell (`index.html`) for a page. Returns the shell unchanged when it is not the app shell.
pub fn apply(shell: &str, p: &Page) -> String {
    if !shell.contains("<div id=\"app\">") {
        return shell.to_string();
    }
    let mut html = shell.to_string();
    if let (Some(a), Some(b)) = (html.find("<title>"), html.find("</title>")) {
        html.replace_range(a + 7..b, &esc(p.title));
    }
    for prefix in [
        "<meta name=\"description\" content=\"",
        "<meta property=\"og:description\" content=\"",
        "<meta name=\"twitter:description\" content=\"",
    ] {
        set_attr(&mut html, prefix, p.description);
    }
    for prefix in [
        "<meta property=\"og:title\" content=\"",
        "<meta name=\"twitter:title\" content=\"",
    ] {
        set_attr(&mut html, prefix, p.title);
    }
    for prefix in [
        "<link rel=\"canonical\" href=\"",
        "<meta property=\"og:url\" content=\"",
    ] {
        set_attr(&mut html, prefix, p.canonical);
    }
    set_attr(
        &mut html,
        "<link rel=\"alternate\" type=\"text/markdown\" href=\"",
        p.markdown_twin,
    );
    // Structured data: this page's own, in place of the generic block in the shell.
    while let Some(a) = html.find("<script type=\"application/ld+json\">") {
        let Some(len) = html[a..].find("</script>") else {
            break;
        };
        html.replace_range(a..a + len + "</script>".len(), "");
    }
    let ld: String = p
        .json_ld
        .iter()
        .map(|b| {
            format!(
                "<script type=\"application/ld+json\">{}</script>\n",
                b.replace("</", "<\\/")
            )
        })
        .collect();
    if let Some(at) = html.find("</head>") {
        html.insert_str(at, &ld);
    }
    // The text crawlers read (the app clears #app when it starts).
    if let (Some(a), Some(b)) = (html.find("<main style="), html.find("</main>")) {
        let main = format!(
            "<main style=\"font-family: system-ui, sans-serif; max-width: 46rem; margin: 3rem auto; padding: 0 1rem\">\n{}<nav><a href=\"/\">Playground</a> · <a href=\"/docs/quickstart\">Quickstart</a> · <a href=\"/docs/api\">REST API</a> · <a href=\"/docs/mcp\">MCP server</a> · <a href=\"/docs/run-it-yourself\">Run it yourself</a> · <a href=\"/about\">About</a></nav>\n<noscript><p>This site needs JavaScript to run the playground.</p></noscript>\n",
            p.body_html
        );
        html.replace_range(a..b, &main);
    }
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_becomes_safe_simple_html() {
        let h = md_to_html(
            "# Title\n\nSome **bold** and `a<b` text with [a link](/docs/api) and [bad](javascript:alert(1)).\n\n- one\n- two\n\n1. first\n2. second\n\n```json\n{\"a\": \"</script>\"}\n```\n",
        );
        assert!(h.contains("<h1>Title</h1>"));
        assert!(h.contains("<strong>bold</strong>") && h.contains("<code>a&lt;b</code>"));
        assert!(h.contains("<a href=\"/docs/api\">a link</a>"));
        assert!(!h.contains("href=\"javascript"));
        assert!(h.contains("<ul>\n<li>one</li>") && h.contains("<ol>\n<li>first</li>"));
        assert!(h.contains("&lt;/script&gt;") && !h.contains("</script>"));
    }

    #[test]
    fn a_page_fills_the_shell() {
        let shell = "<html><head><title>x</title><meta name=\"description\" content=\"old\" /><link rel=\"canonical\" href=\"https://a/\" /><script type=\"application/ld+json\">{}</script></head><body><div id=\"app\"><main style=\"x\">old</main></div></body></html>";
        let ld = vec!["{\"@type\":\"FAQPage\",\"x\":\"</script>\"}".to_string()];
        let out = apply(
            shell,
            &Page {
                title: "T & U",
                description: "D \"q\"",
                canonical: "https://a/about",
                markdown_twin: "https://a/about.md",
                body_html: "<h1>About</h1>\n",
                json_ld: &ld,
            },
        );
        assert!(out.contains("<title>T &amp; U</title>"));
        assert!(out.contains("content=\"D &quot;q&quot;\""));
        assert!(out.contains("href=\"https://a/about\""));
        assert!(out.contains("<h1>About</h1>") && !out.contains(">old</main>"));
        assert_eq!(out.matches("application/ld+json").count(), 1);
        assert!(out.contains("FAQPage") && !out.contains("\"</script>\""));
        // not the app shell: unchanged
        assert_eq!(
            apply(
                "<html>x</html>",
                &Page {
                    title: "",
                    description: "",
                    canonical: "",
                    markdown_twin: "",
                    body_html: "",
                    json_ld: &[]
                }
            ),
            "<html>x</html>"
        );
    }
}
