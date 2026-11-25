//! A parser for the values you type in the MongoDB shell: JSON plus what mongosh allows.
//!
//! Unquoted keys (`{name: "Ada"}`), single-quoted strings, trailing commas, `//` and `/* */`
//! comments, `/regex/flags` literals, and the shell's constructors (`ObjectId("..")`,
//! `ISODate("..")`, `new Date()`, `NumberLong(..)`, `UUID(..)` ...). Everything becomes strict
//! MongoDB Extended JSON, so the rest of the pipeline only sees `serde_json::Value`.
//!
//! The same [`Parser`] reads the arguments of `db.coll.method(...)` calls (see `shell.rs`).
//! Errors carry a line and column.

use serde_json::{json, Map, Number, Value};

use crate::{
    error::PlaygroundError,
    limits::{
        MAX_EXPANDED_BYTES, MAX_REGEX_LITERALS, MAX_REGEX_PATTERN_LEN, MAX_SCRIPT_VARIABLES,
        MAX_VARIABLE_BYTES,
    },
};

/// Names a script variable cannot take: they already mean something in a value.
const RESERVED: &[&str] = &[
    "db",
    "true",
    "false",
    "null",
    "undefined",
    "NaN",
    "Infinity",
    "new",
    "var",
    "let",
    "const",
    "ObjectId",
    "ISODate",
    "Date",
    "NumberInt",
    "NumberLong",
    "NumberDecimal",
    "Decimal128",
    "UUID",
    "Timestamp",
    "MinKey",
    "MaxKey",
];

type Result<T> = std::result::Result<T, PlaygroundError>;

/// Flags MongoDB's PCRE accepts. JavaScript-only flags such as `g` are rejected.
const FLAGS: &str = "imxsu";

pub(crate) struct Parser {
    chars: Vec<char>,
    pub pos: usize,
    what: String,
    regexes: usize,
    /// `var name = value` declarations seen so far, each with its serialized size.
    vars: std::collections::HashMap<String, (Value, usize)>,
    /// Total size of the variable values copied in so far (see `MAX_EXPANDED_BYTES`).
    expanded: usize,
}

fn ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

fn ident_part(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

impl Parser {
    pub fn new(text: &str, what: &str) -> Self {
        Self {
            chars: text.chars().collect(),
            pos: 0,
            what: what.to_string(),
            regexes: 0,
            vars: Default::default(),
            expanded: 0,
        }
    }

    pub fn has_var(&self, name: &str) -> bool {
        self.vars.contains_key(name)
    }

    /// `var name = value` (or a later `name = value`): later values can use `name`.
    pub fn declare(&mut self, at: usize, name: &str, value: Value) -> Result<()> {
        if RESERVED.contains(&name) {
            return self.fail(at, format!("`{name}` cannot be used as a variable name"));
        }
        if !self.vars.contains_key(name) && self.vars.len() >= MAX_SCRIPT_VARIABLES {
            return self.fail(
                at,
                format!("at most {MAX_SCRIPT_VARIABLES} variables are allowed"),
            );
        }
        let size = value.to_string().len();
        if size > MAX_VARIABLE_BYTES {
            return self.fail(
                at,
                format!(
                    "`{name}` is {size} bytes; a variable can hold at most {MAX_VARIABLE_BYTES}"
                ),
            );
        }
        self.vars.insert(name.to_string(), (value, size));
        Ok(())
    }

    pub fn text(&self, from: usize, to: usize) -> String {
        self.chars[from..to.min(self.chars.len())].iter().collect()
    }

    pub fn line(&self, pos: usize) -> usize {
        self.chars[..pos.min(self.chars.len())]
            .iter()
            .filter(|c| **c == '\n')
            .count()
            + 1
    }

    fn column(&self, pos: usize) -> usize {
        self.chars[..pos.min(self.chars.len())]
            .iter()
            .rev()
            .take_while(|c| **c != '\n')
            .count()
            + 1
    }

    pub fn fail<T>(&self, pos: usize, msg: impl Into<String>) -> Result<T> {
        Err(PlaygroundError::Invalid(format!(
            "{}: {} (line {}, column {})",
            self.what,
            msg.into(),
            self.line(pos),
            self.column(pos)
        )))
    }

    /// Skip whitespace and comments.
    pub fn skip(&mut self) -> Result<()> {
        loop {
            match (self.chars.get(self.pos), self.chars.get(self.pos + 1)) {
                (Some(c), _) if c.is_whitespace() => self.pos += 1,
                (Some('/'), Some('/')) => {
                    while self.pos < self.chars.len() && self.chars[self.pos] != '\n' {
                        self.pos += 1;
                    }
                }
                (Some('/'), Some('*')) => {
                    let start = self.pos;
                    self.pos += 2;
                    loop {
                        if self.pos + 1 >= self.chars.len() {
                            return self.fail(start, "unterminated /* comment");
                        }
                        if self.chars[self.pos] == '*' && self.chars[self.pos + 1] == '/' {
                            self.pos += 2;
                            break;
                        }
                        self.pos += 1;
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    pub fn peek(&mut self) -> Result<Option<char>> {
        self.skip()?;
        Ok(self.chars.get(self.pos).copied())
    }

    pub fn eat(&mut self, c: char) -> Result<bool> {
        if self.peek()? == Some(c) {
            self.pos += 1;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn expect(&mut self, c: char) -> Result<()> {
        if self.eat(c)? {
            Ok(())
        } else {
            self.fail(self.pos, format!("expected `{c}`"))
        }
    }

    pub fn ident(&mut self) -> Result<Option<String>> {
        self.skip()?;
        let start = self.pos;
        if !self.chars.get(start).is_some_and(|c| ident_start(*c)) {
            return Ok(None);
        }
        while self.chars.get(self.pos).is_some_and(|c| ident_part(*c)) {
            self.pos += 1;
        }
        Ok(Some(self.text(start, self.pos)))
    }

    /// Arguments of a call whose `(` has been consumed, through the closing `)`.
    pub fn args(&mut self) -> Result<Vec<Value>> {
        let mut args = Vec::new();
        loop {
            if self.eat(')')? {
                break;
            }
            args.push(self.value()?);
            if !self.eat(',')? {
                self.expect(')')?;
                break;
            }
        }
        Ok(args)
    }

    pub fn value(&mut self) -> Result<Value> {
        let Some(c) = self.peek()? else {
            return self.fail(self.pos, "unexpected end of input");
        };
        match c {
            '{' => self.object(),
            '[' => self.array(),
            '"' | '\'' => Ok(Value::String(self.string()?)),
            '/' => self.regex(),
            '-' | '.' => self.number(),
            c if c.is_ascii_digit() => self.number(),
            c if ident_start(c) => self.word(),
            other => self.fail(self.pos, format!("unexpected `{other}`")),
        }
    }

    fn object(&mut self) -> Result<Value> {
        self.pos += 1;
        let mut map = Map::new();
        loop {
            if self.eat('}')? {
                break;
            }
            let key = self.key()?;
            self.expect(':')?;
            let v = self.value()?;
            map.insert(key, v);
            if !self.eat(',')? {
                self.expect('}')?;
                break;
            }
        }
        Ok(Value::Object(map))
    }

    fn key(&mut self) -> Result<String> {
        match self.peek()? {
            Some('"') | Some('\'') => self.string(),
            Some(c) if ident_part(c) => {
                let start = self.pos;
                while self.chars.get(self.pos).is_some_and(|c| ident_part(*c)) {
                    self.pos += 1;
                }
                Ok(self.text(start, self.pos))
            }
            _ => self.fail(
                self.pos,
                "expected a field name (quote it if it has dots or dashes)",
            ),
        }
    }

    fn array(&mut self) -> Result<Value> {
        self.pos += 1;
        let mut items = Vec::new();
        loop {
            if self.eat(']')? {
                break;
            }
            items.push(self.value()?);
            if !self.eat(',')? {
                self.expect(']')?;
                break;
            }
        }
        Ok(Value::Array(items))
    }

    /// A `"..."` or `'...'` string literal at the current position.
    pub fn string(&mut self) -> Result<String> {
        self.skip()?;
        let start = self.pos;
        let quote = match self.chars.get(start) {
            Some(q @ ('"' | '\'')) => *q,
            _ => return self.fail(start, "expected a string"),
        };
        self.pos += 1;
        let mut out = String::new();
        loop {
            let Some(&c) = self.chars.get(self.pos) else {
                return self.fail(start, "unterminated string");
            };
            self.pos += 1;
            match c {
                c if c == quote => break,
                '\n' => return self.fail(start, "unterminated string"),
                '\\' => {
                    let Some(&e) = self.chars.get(self.pos) else {
                        return self.fail(start, "unterminated string");
                    };
                    self.pos += 1;
                    out.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        'b' => '\u{8}',
                        'f' => '\u{c}',
                        'u' => {
                            let hex = self.text(self.pos, self.pos + 4);
                            self.pos += 4;
                            match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                                Some(ch) => ch,
                                None => return self.fail(self.pos, "invalid \\u escape"),
                            }
                        }
                        other => other,
                    });
                }
                c => out.push(c),
            }
        }
        Ok(out)
    }

    fn number(&mut self) -> Result<Value> {
        let start = self.pos;
        while self
            .chars
            .get(self.pos)
            .is_some_and(|c| c.is_ascii_digit() || ".eE+-".contains(*c))
        {
            self.pos += 1;
        }
        let text = self.text(start, self.pos);
        if let Ok(n) = text.parse::<i64>() {
            return Ok(Value::Number(n.into()));
        }
        match text.parse::<f64>().ok().and_then(Number::from_f64) {
            Some(n) => Ok(Value::Number(n)),
            None => self.fail(start, format!("invalid number `{text}`")),
        }
    }

    /// `/pattern/flags` becomes `{"$regularExpression": {"pattern", "options"}}`.
    fn regex(&mut self) -> Result<Value> {
        let start = self.pos;
        self.regexes += 1;
        if self.regexes > MAX_REGEX_LITERALS {
            return self.fail(
                start,
                format!("at most {MAX_REGEX_LITERALS} regex literals are allowed"),
            );
        }
        self.pos += 1;
        let mut pattern = String::new();
        let mut in_class = false;
        let mut closed = false;
        while let Some(&ch) = self.chars.get(self.pos) {
            if ch == '\n' {
                break;
            }
            if ch == '\\' {
                pattern.push(ch);
                self.pos += 1;
                if let Some(&esc) = self.chars.get(self.pos) {
                    if esc != '\n' {
                        pattern.push(esc);
                        self.pos += 1;
                    }
                }
                continue;
            }
            match ch {
                '[' => in_class = true,
                ']' => in_class = false,
                '/' if !in_class => {
                    closed = true;
                    self.pos += 1;
                    break;
                }
                _ => {}
            }
            pattern.push(ch);
            self.pos += 1;
        }
        if !closed {
            return self.fail(start, "unterminated regex literal");
        }
        let mut flags = String::new();
        while self
            .chars
            .get(self.pos)
            .is_some_and(|c| c.is_ascii_alphabetic())
        {
            flags.push(self.chars[self.pos]);
            self.pos += 1;
        }
        if let Some(bad) = flags.chars().find(|c| !FLAGS.contains(*c)) {
            return self.fail(
                start,
                format!("regex flag `{bad}` is not supported; MongoDB accepts: i, m, x, s, u"),
            );
        }
        if pattern.chars().count() > MAX_REGEX_PATTERN_LEN {
            return self.fail(
                start,
                format!("a regex is longer than {MAX_REGEX_PATTERN_LEN} characters"),
            );
        }
        Ok(json!({"$regularExpression": {"pattern": pattern, "options": flags}}))
    }

    /// `true`, `null`, `ObjectId("..")`, `new Date()`, ...
    fn word(&mut self) -> Result<Value> {
        let start = self.pos;
        let mut name = self.ident()?.unwrap_or_default();
        if name == "new" {
            name = match self.ident()? {
                Some(n) => n,
                None => return self.fail(self.pos, "expected a constructor after `new`"),
            };
        }
        if self.peek()? == Some('(') {
            self.pos += 1;
            let args = self.args()?;
            return self.construct(start, &name, &args);
        }
        if let Some((v, size)) = self.vars.get(&name) {
            let total = self.expanded + size;
            if total > MAX_EXPANDED_BYTES {
                return self.fail(
                    start,
                    format!(
                        "the variables used here add up to more than {MAX_EXPANDED_BYTES} bytes"
                    ),
                );
            }
            let v = v.clone();
            self.expanded = total;
            return Ok(v);
        }
        match name.as_str() {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            "null" | "undefined" => Ok(Value::Null),
            "NaN" => Ok(json!({"$numberDouble": "NaN"})),
            "Infinity" => Ok(json!({"$numberDouble": "Infinity"})),
            other => self.fail(
                start,
                format!("unknown value `{other}`; quote strings like \"{other}\", or declare a variable first: var {other} = {{...}}"),
            ),
        }
    }

    fn construct(&self, at: usize, name: &str, args: &[Value]) -> Result<Value> {
        let text = |i: usize| match args.get(i) {
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Number(n)) => Some(n.to_string()),
            _ => None,
        };
        let need = |what: &str| -> Result<String> {
            text(0).map_or_else(|| self.fail(at, format!("{name}() needs {what}")), Ok)
        };
        Ok(match name {
            "ObjectId" => {
                let hex = match args.first() {
                    None => uuid::Uuid::new_v4().simple().to_string()[..24].to_string(),
                    Some(_) => need("a 24-character hex string")?,
                };
                if hex.len() != 24 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                    return self.fail(at, "ObjectId() needs a 24-character hex string");
                }
                json!({"$oid": hex})
            }
            "ISODate" | "Date" => match args.first() {
                None => {
                    let ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis())
                        .unwrap_or(0);
                    json!({"$date": {"$numberLong": ms.to_string()}})
                }
                Some(Value::Number(n)) => json!({"$date": {"$numberLong": n.to_string()}}),
                Some(_) => json!({"$date": iso_date(&need("an ISO date string")?)}),
            },
            "NumberInt" => json!({"$numberInt": need("a number")?}),
            "NumberLong" => json!({"$numberLong": need("a number")?}),
            "NumberDecimal" | "Decimal128" => json!({"$numberDecimal": need("a decimal string")?}),
            "UUID" => json!({"$uuid": need("a UUID string")?}),
            "Timestamp" => match (
                args.first().and_then(Value::as_i64),
                args.get(1).and_then(Value::as_i64),
            ) {
                (Some(t), Some(i)) => json!({"$timestamp": {"t": t, "i": i}}),
                _ => return self.fail(at, "Timestamp(t, i) needs two integers"),
            },
            "MinKey" => json!({"$minKey": 1}),
            "MaxKey" => json!({"$maxKey": 1}),
            other => return self.fail(at, format!("unknown function `{other}()`")),
        })
    }
}

/// The shell is forgiving about dates ("2024-01-01", "2024-01-01T10:00:00"); BSON wants RFC 3339 in UTC.
fn iso_date(s: &str) -> String {
    let t = s.trim();
    let date_only = t.len() == 10
        && t.chars().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                c == '-'
            } else {
                c.is_ascii_digit()
            }
        });
    if date_only {
        return format!("{t}T00:00:00Z");
    }
    match t.split_once('T') {
        Some((_, time)) if !time.ends_with('Z') && !time.contains('+') && !time.contains('-') => {
            format!("{t}Z")
        }
        _ => t.to_string(),
    }
}

/// Parse text that holds exactly one value.
pub(crate) fn parse_value(text: &str, what: &str) -> Result<Value> {
    let mut p = Parser::new(text, what);
    let v = p.value()?;
    // A trailing `;` is fine (it is what people paste from the shell).
    p.eat(';')?;
    if p.peek()?.is_some() {
        return p.fail(p.pos, "unexpected text after the value");
    }
    Ok(v)
}

/// The documents editor: a value, optionally written `db = {...}` like the playground sites do.
pub(crate) fn parse_config(text: &str, what: &str) -> Result<Value> {
    let mut p = Parser::new(text, what);
    p.skip()?;
    let save = p.pos;
    if !(p.ident()?.as_deref() == Some("db") && p.eat('=')?) {
        p.pos = save;
    }
    let v = p.value()?;
    p.eat(';')?;
    if p.peek()?.is_some() {
        return p.fail(p.pos, "unexpected text after the value");
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn regex(pattern: &str, options: &str) -> Value {
        json!({"$regularExpression": {"pattern": pattern, "options": options}})
    }

    fn p(src: &str) -> Value {
        parse_value(src, "q").unwrap_or_else(|e| panic!("{src}: {e}"))
    }

    #[test]
    fn strict_json_parses_unchanged() {
        let src = r#"{"a": [1, 2.5, "x/y", null, true], "b": {"c": "//not a comment"}}"#;
        assert_eq!(p(src), serde_json::from_str::<Value>(src).unwrap());
    }

    #[test]
    fn shell_syntax_unquoted_keys_single_quotes_trailing_commas() {
        assert_eq!(
            p("{name: 'Ada', tags: ['a', 'b',], $or: [{age: {$gt: 30}},],}"),
            json!({"name": "Ada", "tags": ["a", "b"], "$or": [{"age": {"$gt": 30}}]})
        );
        assert_eq!(
            p(r#"{ "a.b": 1, 'c-d': 2, 3: "x" }"#),
            json!({"a.b": 1, "c-d": 2, "3": "x"})
        );
        assert_eq!(p("{a: undefined}"), json!({"a": null}));
    }

    #[test]
    fn numbers() {
        assert_eq!(
            p("[1, -2, 3.5, 1e3, -0.25, 9007199254740993]"),
            json!([1, -2, 3.5, 1000.0, -0.25, 9007199254740993_i64])
        );
    }

    #[test]
    fn string_escapes() {
        assert_eq!(p(r#"'it\'s \"x\" \n A'"#), json!("it's \"x\" \n A"));
    }

    #[test]
    fn constructors_become_extended_json() {
        assert_eq!(
            p(r#"ObjectId("507f1f77bcf86cd799439011")"#),
            json!({"$oid": "507f1f77bcf86cd799439011"})
        );
        assert_eq!(
            p(r#"ISODate("2024-01-01T00:00:00Z")"#),
            json!({"$date": "2024-01-01T00:00:00Z"})
        );
        assert_eq!(
            p(r#"new Date("2024-01-01")"#),
            json!({"$date": "2024-01-01T00:00:00Z"})
        );
        assert_eq!(p("NumberLong(5)"), json!({"$numberLong": "5"}));
        assert_eq!(
            p(r#"NumberDecimal("1.50")"#),
            json!({"$numberDecimal": "1.50"})
        );
        assert_eq!(
            p("Timestamp(1, 2)"),
            json!({"$timestamp": {"t": 1, "i": 2}})
        );
        assert!(p("ObjectId()")["$oid"].as_str().unwrap().len() == 24);
        assert!(p("new Date()")["$date"]["$numberLong"].is_string());
        assert!(parse_value("ObjectId('xyz')", "q").is_err());
        assert!(parse_value("Nope(1)", "q").is_err());
        assert!(parse_value("{a: bareword}", "q")
            .unwrap_err()
            .to_string()
            .contains("quote strings"));
    }

    #[test]
    fn constructors_are_valid_bson() {
        let v = p(
            r#"{_id: ObjectId("507f1f77bcf86cd799439011"), at: ISODate("2024-01-01T00:00:00Z"), n: NumberLong(7), u: UUID("123e4567-e89b-12d3-a456-426614174000")}"#,
        );
        let b = mongodb::bson::Bson::try_from(v).unwrap();
        let d = b.as_document().unwrap();
        assert!(matches!(
            d.get("_id"),
            Some(mongodb::bson::Bson::ObjectId(_))
        ));
        assert!(matches!(
            d.get("at"),
            Some(mongodb::bson::Bson::DateTime(_))
        ));
        assert!(matches!(d.get("n"), Some(mongodb::bson::Bson::Int64(7))));
    }

    #[test]
    fn regex_literals() {
        assert_eq!(p("{name: /^ada/i}"), json!({"name": regex("^ada", "i")}));
        assert_eq!(p("[/a/, /b/ms]"), json!([regex("a", ""), regex("b", "ms")]));
        assert_eq!(p(r"{a: /[/]x/}"), json!({"a": regex("[/]x", "")}));
        assert_eq!(p(r"{a: /a\/b/}"), json!({"a": regex(r"a\/b", "")}));
        assert_eq!(p(r#"{a: /\d+"q"/}"#), json!({"a": regex(r#"\d+"q""#, "")}));
        assert_eq!(
            p(r#"{a: "/^ada/i"}"#),
            json!({"a": "/^ada/i"}),
            "inside strings it is just text"
        );
    }

    #[test]
    fn regex_errors_name_the_problem_and_line() {
        let e = parse_value("{a: /x/g}", "q").unwrap_err().to_string();
        assert!(e.contains("`g`") && e.contains("line 1"), "{e}");
        let e = parse_value("{\n a: 1,\n b: /abc\n}", "q")
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("unterminated regex") && e.contains("line 3"),
            "{e}"
        );
        assert!(parse_value(&format!("{{a: /{}/}}", "x".repeat(257)), "q").is_err());
        assert!(parse_value(&format!("{{a: /{}/}}", "x".repeat(256)), "q").is_ok());
    }

    #[test]
    fn at_most_twenty_regex_literals() {
        let many = |n: usize| format!("[{}]", vec!["/a/"; n].join(","));
        assert!(parse_value(&many(20), "q").is_ok());
        assert!(parse_value(&many(21), "q").is_err());
    }

    #[test]
    fn comments_and_trailing_semicolon() {
        assert_eq!(
            p("{ // line\n a: /* block */ 1 } // end\n;"),
            json!({"a": 1})
        );
        assert!(parse_value("{a: /* never closed", "q").is_err());
    }

    #[test]
    fn errors_have_line_and_column() {
        let e = parse_value("{\n  a: 1,\n  b: ,\n}", "q")
            .unwrap_err()
            .to_string();
        assert!(e.contains("line 3"), "{e}");
        assert!(parse_value("{a: 1} extra", "q").is_err());
        assert!(parse_value("{a 1}", "q").is_err());
        assert!(parse_value("", "q").is_err());
        assert!(parse_value("1 / 2", "q").is_err());
    }

    #[test]
    fn config_accepts_db_assignment_and_plain_values() {
        let a = parse_config("db = { people: [{name: 'Ada'}] };", "config").unwrap();
        assert_eq!(a, json!({"people": [{"name": "Ada"}]}));
        assert_eq!(
            parse_config("db={a: []}", "config").unwrap(),
            json!({"a": []})
        );
        assert_eq!(
            parse_config("[{x: 1}]", "config").unwrap(),
            json!([{"x": 1}])
        );
    }

    #[test]
    fn forgiving_date_strings() {
        assert_eq!(
            p(r#"new Date("2024-01-01")"#),
            json!({"$date": "2024-01-01T00:00:00Z"})
        );
        assert_eq!(
            p(r#"ISODate("2024-01-01T10:30:00")"#),
            json!({"$date": "2024-01-01T10:30:00Z"})
        );
        assert_eq!(
            p(r#"ISODate("2024-01-01T10:30:00+02:00")"#),
            json!({"$date": "2024-01-01T10:30:00+02:00"})
        );
        assert_eq!(
            p(r#"ISODate("2024-01-01T10:30:00Z")"#),
            json!({"$date": "2024-01-01T10:30:00Z"})
        );
    }
}
