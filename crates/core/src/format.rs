//! Tidy text: re-indent and re-wrap the Data panel (relaxed JSON) and shell scripts (mongosh).
//!
//! This is a layout pass, not a parser: it reads the text as tokens (strings, regex literals, comments, brackets,
//! commas, words), groups the brackets, and writes the same tokens back with consistent spacing. Short objects,
//! arrays and calls stay on one line; ones that do not fit the width are broken one item per line. Comments stay
//! where they were, and nothing is added or removed except whitespace and trailing commas. Before returning, the
//! result is re-read and compared with the input token by token, so a bug here can never change a query.

use crate::error::PlaygroundError;

type Result<T> = std::result::Result<T, PlaygroundError>;

pub const DEFAULT_WIDTH: usize = 80;
pub const MIN_WIDTH: usize = 20;
pub const MAX_WIDTH: usize = 200;
const INDENT: &str = "  ";

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Str(String),
    Regex(String),
    Open(char),
    Close(char),
    Comma,
    Colon,
    Semi,
    Line(String),
    Block(String),
}

#[derive(Debug, Clone)]
struct Item {
    tok: Tok,
    /// Whitespace (without a line break) came before this token.
    space: bool,
    /// How many line breaks came before it.
    newlines: usize,
    /// 1-based line, for error messages.
    line: usize,
}

fn invalid(msg: impl Into<String>) -> PlaygroundError {
    PlaygroundError::Invalid(msg.into())
}

// ----- reading ------------------------------------------------------------------------------------------------

fn tokenize(text: &str) -> Result<Vec<Item>> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<Item> = vec![];
    let (mut i, mut line) = (0usize, 1usize);
    let (mut space, mut newlines) = (false, 0usize);

    let regex_allowed = |out: &[Item]| match out.last().map(|t| &t.tok) {
        None => true,
        Some(Tok::Open(_) | Tok::Comma | Tok::Colon | Tok::Semi) => true,
        Some(Tok::Word(w)) => w.ends_with(['=', '!', '<', '>', '&', '|', '?']),
        _ => false,
    };

    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            space = true;
            if c == '\n' {
                newlines += 1;
                line += 1;
            }
            i += 1;
            continue;
        }
        let start_line = line;
        let tok = match c {
            '/' if chars.get(i + 1) == Some(&'/') => {
                let from = i;
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                Tok::Line(
                    chars[from..i]
                        .iter()
                        .collect::<String>()
                        .trim_end()
                        .to_string(),
                )
            }
            '/' if chars.get(i + 1) == Some(&'*') => {
                let from = i;
                i += 2;
                loop {
                    if i + 1 >= chars.len() {
                        return Err(invalid(format!(
                            "line {start_line}: a /* comment is never closed"
                        )));
                    }
                    if chars[i] == '*' && chars[i + 1] == '/' {
                        i += 2;
                        break;
                    }
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
                Tok::Block(chars[from..i].iter().collect())
            }
            '/' if regex_allowed(&out) && regex_end(&chars, i).is_some() => {
                let end = regex_end(&chars, i).unwrap_or(i + 1);
                let s: String = chars[i..end].iter().collect();
                i = end;
                Tok::Regex(s)
            }
            '\'' | '"' | '`' => {
                let from = i;
                i += 1;
                loop {
                    let Some(&d) = chars.get(i) else {
                        return Err(invalid(format!(
                            "line {start_line}: a string is never closed"
                        )));
                    };
                    if d == '\\' {
                        i += 2;
                        continue;
                    }
                    if d == '\n' {
                        line += 1;
                        if c != '`' {
                            return Err(invalid(format!(
                                "line {start_line}: a string is never closed"
                            )));
                        }
                    }
                    i += 1;
                    if d == c {
                        break;
                    }
                }
                Tok::Str(chars[from..i.min(chars.len())].iter().collect())
            }
            '{' | '[' | '(' => {
                i += 1;
                Tok::Open(c)
            }
            '}' | ']' | ')' => {
                i += 1;
                Tok::Close(c)
            }
            ',' => {
                i += 1;
                Tok::Comma
            }
            ':' => {
                i += 1;
                Tok::Colon
            }
            ';' => {
                i += 1;
                Tok::Semi
            }
            _ => {
                let from = i;
                while i < chars.len() {
                    let d = chars[i];
                    let comment = d == '/' && matches!(chars.get(i + 1), Some('/') | Some('*'));
                    if d.is_whitespace() || "{}[](),:;'\"`".contains(d) || comment {
                        break;
                    }
                    i += 1;
                }
                Tok::Word(chars[from..i].iter().collect())
            }
        };
        out.push(Item {
            tok,
            space: space || newlines > 0,
            newlines,
            line: start_line,
        });
        space = false;
        newlines = 0;
    }
    Ok(out)
}

/// Index just past a regex literal starting at `start` (a `/`), or None when it is not one (no closing `/` on the line).
fn regex_end(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start + 1;
    let mut in_class = false;
    while i < chars.len() {
        match chars[i] {
            '\n' => return None,
            '\\' => i += 1,
            '[' => in_class = true,
            ']' => in_class = false,
            '/' if !in_class => {
                i += 1;
                while i < chars.len() && chars[i].is_ascii_alphabetic() {
                    i += 1;
                }
                return (i > start + 1).then_some(i);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

// ----- structure ----------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Node {
    Leaf(Item),
    Group(Group),
}

#[derive(Debug, Clone)]
struct Group {
    open: char,
    close: char,
    space: bool,
    newlines: usize,
    line: usize,
    items: Vec<Node>,
}

impl Node {
    fn space(&self) -> bool {
        match self {
            Node::Leaf(i) => i.space,
            Node::Group(g) => g.space,
        }
    }
    fn newlines(&self) -> usize {
        match self {
            Node::Leaf(i) => i.newlines,
            Node::Group(g) => g.newlines,
        }
    }
}

fn closer(open: char) -> char {
    match open {
        '{' => '}',
        '[' => ']',
        _ => ')',
    }
}

fn build(items: Vec<Item>) -> Result<Vec<Node>> {
    let mut stack: Vec<Group> = vec![];
    let mut top: Vec<Node> = vec![];
    for it in items {
        match it.tok {
            Tok::Open(c) => stack.push(Group {
                open: c,
                close: closer(c),
                space: it.space,
                newlines: it.newlines,
                line: it.line,
                items: vec![],
            }),
            Tok::Close(c) => {
                let Some(g) = stack.pop() else {
                    return Err(invalid(format!(
                        "line {}: `{c}` closes something that was never opened",
                        it.line
                    )));
                };
                if g.close != c {
                    return Err(invalid(format!(
                        "line {}: `{c}` does not match the `{}` opened on line {}",
                        it.line, g.open, g.line
                    )));
                }
                match stack.last_mut() {
                    Some(parent) => parent.items.push(Node::Group(g)),
                    None => top.push(Node::Group(g)),
                }
            }
            _ => match stack.last_mut() {
                Some(parent) => parent.items.push(Node::Leaf(it)),
                None => top.push(Node::Leaf(it)),
            },
        }
    }
    if let Some(g) = stack.last() {
        return Err(invalid(format!(
            "line {}: `{}` is never closed",
            g.line, g.open
        )));
    }
    Ok(top)
}

// ----- writing ------------------------------------------------------------------------------------------------

fn tok_text(t: &Tok) -> &str {
    match t {
        Tok::Word(s) | Tok::Str(s) | Tok::Regex(s) | Tok::Line(s) | Tok::Block(s) => s,
        Tok::Comma => ",",
        Tok::Colon => ":",
        Tok::Semi => ";",
        Tok::Open(_) | Tok::Close(_) => "",
    }
}

fn is_comment(n: &Node) -> bool {
    matches!(
        n,
        Node::Leaf(Item {
            tok: Tok::Line(_) | Tok::Block(_),
            ..
        })
    )
}

fn is_tok(n: &Node, t: &Tok) -> bool {
    matches!(n, Node::Leaf(i) if &i.tok == t)
}

/// The text between two neighbours when they sit on one line.
fn gap(prev: Option<&Node>, cur: &Node) -> &'static str {
    let Some(prev) = prev else { return "" };
    if matches!(
        cur,
        Node::Leaf(Item {
            tok: Tok::Comma | Tok::Colon | Tok::Semi,
            ..
        })
    ) {
        return "";
    }
    if is_tok(prev, &Tok::Comma) || is_tok(prev, &Tok::Colon) {
        return " ";
    }
    if cur.space() {
        " "
    } else {
        ""
    }
}

/// A group written on one line, or None when it cannot be (a line comment, or a multi-line block comment, inside).
fn flat_group(g: &Group) -> Option<String> {
    let inner = flat_seq(&g.items)?;
    Some(if g.open == '{' && !inner.is_empty() {
        format!("{{ {inner} }}")
    } else {
        format!("{}{inner}{}", g.open, g.close)
    })
}

fn flat_seq(nodes: &[Node]) -> Option<String> {
    let mut s = String::new();
    let mut prev: Option<&Node> = None;
    let last_real = nodes.iter().rposition(|n| !is_tok(n, &Tok::Comma));
    for (i, n) in nodes.iter().enumerate() {
        // A comma right before the closing bracket is dropped.
        if is_tok(n, &Tok::Comma) && last_real.is_none_or(|l| i > l) {
            continue;
        }
        s.push_str(gap(prev, n));
        match n {
            Node::Leaf(it) => match &it.tok {
                Tok::Line(_) => return None,
                Tok::Block(b) if b.contains('\n') => return None,
                t => s.push_str(tok_text(t)),
            },
            Node::Group(g) => s.push_str(&flat_group(g)?),
        }
        prev = Some(n);
    }
    Some(s)
}

struct Writer {
    out: String,
    width: usize,
}

impl Writer {
    fn col(&self) -> usize {
        self.out
            .rsplit('\n')
            .next()
            .map_or(0, |l| l.chars().count())
    }

    fn newline(&mut self, indent: usize) {
        while self.out.ends_with(' ') {
            self.out.pop();
        }
        self.out.push('\n');
        self.out.push_str(&INDENT.repeat(indent));
    }

    fn group(&mut self, g: &Group, indent: usize) {
        if let Some(flat) = flat_group(g) {
            if self.col() + flat.chars().count() + 2 <= self.width {
                self.out.push_str(&flat);
                return;
            }
        }
        // f({ ... }) and f([ ... ]): the one argument opens on the same line as the call.
        let real: Vec<&Node> = g.items.iter().filter(|n| !is_tok(n, &Tok::Comma)).collect();
        if g.open == '(' && real.len() == 1 && !g.items.iter().any(is_comment) {
            if let Node::Group(inner) = real[0] {
                self.out.push('(');
                self.group(inner, indent);
                self.out.push(')');
                return;
            }
        }
        self.out.push(g.open);
        let entries = entries(&g.items);
        // Only items that are followed by another item get a comma: comments after the last item do not make it
        // a middle one.
        let last = entries
            .iter()
            .rposition(|e| !e.nodes.is_empty())
            .unwrap_or(0);
        for (n, e) in entries.iter().enumerate() {
            if n > 0 && e.blank_before {
                self.out.push('\n');
            }
            for c in &e.leading {
                self.newline(indent + 1);
                self.out.push_str(c);
            }
            if !e.nodes.is_empty() {
                self.newline(indent + 1);
                self.seq(&e.nodes, indent + 1);
                if n < last {
                    self.out.push(',');
                }
            }
            if let Some(c) = &e.trailing {
                self.out.push(' ');
                self.out.push_str(c);
            }
        }
        if !g.items.is_empty() {
            self.newline(indent);
        }
        self.out.push(g.close);
    }

    fn seq(&mut self, nodes: &[Node], indent: usize) {
        let mut prev: Option<&Node> = None;
        for n in nodes {
            self.out.push_str(gap(prev, n));
            match n {
                Node::Leaf(it) => self.out.push_str(tok_text(&it.tok)),
                Node::Group(g) => self.group(g, indent),
            }
            prev = Some(n);
        }
    }
}

/// One item of a broken group: the nodes between commas, with the comments that belong to it.
struct Entry {
    leading: Vec<String>,
    nodes: Vec<Node>,
    trailing: Option<String>,
    blank_before: bool,
}

fn entries(items: &[Node]) -> Vec<Entry> {
    let mut out: Vec<Entry> = vec![];
    let mut cur = Entry {
        leading: vec![],
        nodes: vec![],
        trailing: None,
        blank_before: false,
    };
    let mut first_in_entry = true;
    let mut seen_code = false; // any token earlier on the same line as a comment makes it a trailing comment
    let close_entry = |cur: &mut Entry, out: &mut Vec<Entry>| {
        out.push(std::mem::replace(
            cur,
            Entry {
                leading: vec![],
                nodes: vec![],
                trailing: None,
                blank_before: false,
            },
        ));
    };
    for n in items {
        if is_tok(n, &Tok::Comma) {
            close_entry(&mut cur, &mut out);
            first_in_entry = true;
            seen_code = true;
            continue;
        }
        if let Node::Leaf(Item {
            tok: Tok::Line(t) | Tok::Block(t),
            newlines,
            ..
        }) = n
        {
            let own_line = *newlines > 0 || !seen_code;
            if own_line && cur.nodes.is_empty() {
                cur.leading.push(t.clone());
            } else if own_line {
                // A comment on its own line after the item (before the comma): it opens the next entry.
                close_entry(&mut cur, &mut out);
                cur.leading.push(t.clone());
                first_in_entry = false;
            } else if cur.nodes.is_empty() && !out.is_empty() {
                // after a comma, on the same line: it belongs to the item before
                let last = out.len() - 1;
                if out[last].trailing.is_none() {
                    out[last].trailing = Some(t.clone());
                } else {
                    cur.leading.push(t.clone());
                }
            } else {
                cur.trailing = Some(t.clone());
            }
            continue;
        }
        if first_in_entry && cur.nodes.is_empty() {
            cur.blank_before = n.newlines() >= 2;
        }
        first_in_entry = false;
        seen_code = true;
        cur.nodes.push(n.clone());
    }
    if !cur.nodes.is_empty() || !cur.leading.is_empty() || cur.trailing.is_some() {
        out.push(cur);
    }
    out
}

fn write_top(nodes: &[Node], width: usize) -> String {
    let mut w = Writer {
        out: String::new(),
        width,
    };
    let mut prev: Option<&Node> = None;
    for n in nodes {
        match prev {
            None => {}
            Some(_) if n.newlines() > 0 => {
                if n.newlines() >= 2 {
                    w.out.push('\n');
                }
                // A chained call on the next line (`.sort(...)`) stays indented under its statement.
                let chain =
                    matches!(n, Node::Leaf(Item { tok: Tok::Word(s), .. }) if s.starts_with('.'));
                w.newline(usize::from(chain));
            }
            Some(p) => w.out.push_str(gap(Some(p), n)),
        }
        match n {
            Node::Leaf(it) => w.out.push_str(tok_text(&it.tok)),
            Node::Group(g) => {
                let indent = 0;
                w.group(g, indent);
            }
        }
        prev = Some(n);
    }
    let mut s = w.out.trim_end().to_string();
    s.push('\n');
    s
}

// ----- safety -------------------------------------------------------------------------------------------------

/// The tokens that matter, without whitespace and without commas before a closing bracket.
fn signature(text: &str) -> Result<Vec<String>> {
    let items = tokenize(text)?;
    let mut sig: Vec<String> = vec![];
    for (i, it) in items.iter().enumerate() {
        if it.tok == Tok::Comma && matches!(items.get(i + 1).map(|n| &n.tok), Some(Tok::Close(_))) {
            continue;
        }
        sig.push(match &it.tok {
            Tok::Open(c) | Tok::Close(c) => c.to_string(),
            t => tok_text(t).to_string(),
        });
    }
    Ok(sig)
}

/// Tidy `text` to roughly `width` columns. The result always ends with one newline and keeps every token (and
/// comment) of the input. Errors name the line of an unclosed string, comment or bracket.
pub fn tidy(text: &str, width: usize) -> Result<String> {
    let width = width.clamp(MIN_WIDTH, MAX_WIDTH);
    if text.trim().is_empty() {
        return Ok(String::new());
    }
    let nodes = build(tokenize(text)?)?;
    let result = write_top(&nodes, width);
    if signature(text)? != signature(&result)? {
        return Err(invalid(
            "this text could not be tidied safely, so it was left as it is",
        ));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> String {
        tidy(s, 80).unwrap()
    }

    #[test]
    fn short_things_stay_on_one_line_with_consistent_spacing() {
        assert_eq!(
            t("db.people.find({age:{$gt:40}},{_id:0})"),
            "db.people.find({ age: { $gt: 40 } }, { _id: 0 })\n"
        );
        assert_eq!(
            t("db.c.find( { a : 1 , b : [1,2 ,3] } )"),
            "db.c.find({ a: 1, b: [1, 2, 3] })\n"
        );
    }

    #[test]
    fn long_things_break_one_item_per_line() {
        let out = t(
            r#"db.people.insertMany([{name:"Ada",age:36,city:"London"},{name:"Linus",age:55,city:"Helsinki"},{name:"Grace",age:85,city:"New York"}])"#,
        );
        assert_eq!(
            out,
            "db.people.insertMany([\n  { name: \"Ada\", age: 36, city: \"London\" },\n  { name: \"Linus\", age: 55, city: \"Helsinki\" },\n  { name: \"Grace\", age: 85, city: \"New York\" }\n])\n"
        );
    }

    #[test]
    fn the_data_panel_is_tidied_and_comments_stay_put() {
        let out = t("db = {\n collection: {\n documents: [\n {name:\"Ada\",age:36}, // the first\n {name:\"Linus\"}\n ],\n // up to 3\n indexes: [ { key: { age: 1 } } ]\n }\n}");
        assert_eq!(
            out,
            "db = {\n  collection: {\n    documents: [\n      { name: \"Ada\", age: 36 }, // the first\n      { name: \"Linus\" }\n    ],\n    // up to 3\n    indexes: [{ key: { age: 1 } }]\n  }\n}\n"
        );
    }

    #[test]
    fn statements_keep_their_lines_and_chains_are_indented() {
        let out = t("db.a.find({x:1})\n\n\n\ndb.a.find({y:2})\n.sort({y:-1})\n  .limit(5)");
        assert_eq!(
            out,
            "db.a.find({ x: 1 })\n\ndb.a.find({ y: 2 })\n  .sort({ y: -1 })\n  .limit(5)\n"
        );
    }

    #[test]
    fn strings_regexes_and_comments_are_not_touched() {
        let src =
            "db.c.find({ name: /^a.*b/i, url: \"http://x.y/z\", q: 'it\\'s', t: `a\n b` }) // done";
        let out = t(src);
        assert!(
            out.contains("/^a.*b/i")
                && out.contains("\"http://x.y/z\"")
                && out.contains("'it\\'s'")
        );
        assert!(out.contains("`a\n b`") && out.trim_end().ends_with("// done"));
    }

    #[test]
    fn trailing_commas_go_and_strict_json_stays_valid() {
        assert_eq!(
            t("{ \"a\": 1, \"b\": [1, 2,], }"),
            "{ \"a\": 1, \"b\": [1, 2] }\n"
        );
        let json = t("{\"a\":1,\"b\":{\"c\":[true,false,null]}}");
        assert!(serde_json::from_str::<serde_json::Value>(&json).is_ok());
    }

    #[test]
    fn it_is_idempotent() {
        for src in [
            "db.c.aggregate([{$match:{a:1}},{$group:{_id:\"$b\",n:{$sum:1}}},{$sort:{n:-1}},{$limit:5},{$project:{_id:0,b:\"$_id\",n:1}}])",
            "db = { a: { documents: [ { x: 1 }, { x: 2 } ], // c\n indexes: [] } }",
            "db.x.find({})\n.sort({a:1})",
        ] {
            let once = t(src);
            assert_eq!(t(&once), once, "{src}");
        }
    }

    #[test]
    fn variable_declarations_are_tidied_like_any_other_statement() {
        let src = "var SIMPLE_GROUP = {\n    \"$group\": \n      {\n          \"_id\": \"$external_id\",\n          \"count\": { \"$sum\": 1 }\n       }\n    }\nvar MATCH =  { \"$match\": { \"count\": { \"$gt\": 1 } } };\n\n// same\ndb.collection.aggregate(SIMPLE_GROUP, MATCH)";
        let out = t(src);
        assert_eq!(
            out,
            "var SIMPLE_GROUP = {\n  \"$group\": { \"_id\": \"$external_id\", \"count\": { \"$sum\": 1 } }\n}\nvar MATCH = { \"$match\": { \"count\": { \"$gt\": 1 } } };\n\n// same\ndb.collection.aggregate(SIMPLE_GROUP, MATCH)\n"
        );
        assert_eq!(t(&out), out);
    }

    #[test]
    fn comments_after_the_last_item_do_not_add_a_comma() {
        let src = "db = {\n  c: {\n    indexes: [ { key: { a: 1 } } ]\n    // searchIndexes: [],\n    // more\n  }\n}";
        let out = t(src);
        assert!(
            out.contains("indexes: [{ key: { a: 1 } }]\n    // searchIndexes"),
            "{out}"
        );
        assert_eq!(t(&out), out);
    }

    #[test]
    fn narrower_widths_break_sooner() {
        let src = "db.c.find({ aaaa: 1, bbbb: 2, cccc: 3 })";
        assert_eq!(
            tidy(src, 80).unwrap(),
            "db.c.find({ aaaa: 1, bbbb: 2, cccc: 3 })\n"
        );
        assert_eq!(
            tidy(src, 30).unwrap(),
            "db.c.find({\n  aaaa: 1,\n  bbbb: 2,\n  cccc: 3\n})\n"
        );
    }

    #[test]
    fn broken_text_is_refused_with_a_line_number() {
        for (src, needle) in [
            ("db.c.find({ a: 1 )", "does not match"),
            ("db.c.find({ a: \"x })", "string is never closed"),
            ("db.c.find({ a: 1 }) /* open", "comment is never closed"),
            ("db.c.find({ a: 1 }", "never closed"),
            ("a: 1 }", "never opened"),
        ] {
            let e = tidy(src, 80).unwrap_err().to_string();
            assert!(e.contains(needle), "{src}: {e}");
            assert!(e.contains("line 1"), "{src}: {e}");
        }
    }

    #[test]
    fn empty_input_is_empty_output() {
        assert_eq!(t("  \n "), "");
    }
}

#[cfg(test)]
mod example_tests {
    use super::*;

    /// Every shipped template and example: tidying succeeds, and tidying the result again changes nothing.
    #[test]
    fn every_example_is_tidied_safely_and_stays_tidy() {
        let examples = crate::examples();
        for e in examples.as_array().unwrap() {
            for key in ["config", "script"] {
                let Some(text) = e[key].as_str() else {
                    continue;
                };
                let id = e["id"].as_str().unwrap_or("?");
                let once = tidy(text, 80).unwrap_or_else(|err| panic!("{id}.{key}: {err}"));
                let twice = tidy(&once, 80).unwrap();
                assert_eq!(
                    twice, once,
                    "{id}.{key} is not stable:\n--- once\n{once}\n--- twice\n{twice}"
                );
            }
        }
    }
}
