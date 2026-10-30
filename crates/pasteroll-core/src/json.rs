//! The small JSON reader and writer the MCP server needs.
//!
//! MCP speaks JSON-RPC over lines of JSON. Pasteroll has no JSON dependency (its only parser is
//! TOML), and the messages are simple, so this module keeps the server self-contained. It is a
//! strict reader: trailing commas, unquoted keys, lone surrogates and trailing text are errors.
//! Values are written compactly; UTF-8 is kept as UTF-8.

use std::fmt::Write as _;

/// A JSON value. Objects keep their fields in order (the protocol does not care, but the tests
/// and a person reading a log do).
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// A whole, non-negative number (`true` for 3.0, `false` for 3.5 and for anything else).
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Json::Number(n) if n.is_finite() && *n >= 0.0 && n.fract() == 0.0 && *n <= u64::MAX as f64 => Some(*n as u64),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }
}

/// Makes an object from fields (a convenience for the server's replies).
pub fn object(fields: Vec<(&str, Json)>) -> Json {
    Json::Object(fields.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

/// Makes an array.
pub fn array(items: Vec<Json>) -> Json {
    Json::Array(items)
}

/// Makes a string.
pub fn string(text: impl Into<String>) -> Json {
    Json::String(text.into())
}

/// Reads one JSON value, with no trailing text but whitespace.
pub fn parse(text: &str) -> Result<Json, String> {
    let mut p = Parser { bytes: text.as_bytes(), at: 0, depth: 0 };
    p.skip_space();
    let value = p.value()?;
    p.skip_space();
    if p.at != p.bytes.len() {
        return Err(format!("trailing text at byte {}", p.at));
    }
    Ok(value)
}

/// Writes one JSON value compactly.
pub fn stringify(value: &Json) -> String {
    let mut out = String::new();
    write_value(&mut out, value);
    out
}

fn write_value(out: &mut String, value: &Json) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Number(n) => {
            if !n.is_finite() {
                out.push_str("null");
            } else if n.fract() == 0.0 && n.abs() < 9.0e15 {
                let _ = write!(out, "{}", *n as i64);
            } else {
                let _ = write!(out, "{n}");
            }
        }
        Json::String(s) => write_string(out, s),
        Json::Array(items) => {
            out.push('[');
            for (i, v) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, v);
            }
            out.push(']');
        }
        Json::Object(fields) => {
            out.push('{');
            for (i, (k, v)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(out, k);
                out.push(':');
                write_value(out, v);
            }
            out.push('}');
        }
    }
}

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
    depth: usize,
}

const DEPTH_MAX: usize = 128;

impl Parser<'_> {
    fn skip_space(&mut self) {
        while self.at < self.bytes.len() && matches!(self.bytes[self.at], b' ' | b'\t' | b'\r' | b'\n') {
            self.at += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn value(&mut self) -> Result<Json, String> {
        self.depth += 1;
        if self.depth > DEPTH_MAX {
            return Err("JSON is nested too deeply".into());
        }
        let out = match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => self.string().map(Json::String),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(format!("unexpected byte at {}", self.at)),
            None => Err("unexpected end of JSON".into()),
        };
        self.depth -= 1;
        out
    }

    fn literal(&mut self, text: &str, value: Json) -> Result<Json, String> {
        if self.bytes[self.at..].starts_with(text.as_bytes()) {
            self.at += text.len();
            Ok(value)
        } else {
            Err(format!("unexpected text at byte {}", self.at))
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.at += 1;
        let mut fields = Vec::new();
        self.skip_space();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            self.skip_space();
            if self.peek() != Some(b'"') {
                return Err(format!("object key must be a string at byte {}", self.at));
            }
            let key = self.string()?;
            self.skip_space();
            if self.peek() != Some(b':') {
                return Err(format!("expected ':' at byte {}", self.at));
            }
            self.at += 1;
            self.skip_space();
            let value = self.value()?;
            fields.push((key, value));
            self.skip_space();
            match self.peek() {
                Some(b',') => {
                    self.at += 1;
                }
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Json::Object(fields));
                }
                _ => return Err(format!("expected ',' or '}}' at byte {}", self.at)),
            }
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.at += 1;
        let mut items = Vec::new();
        self.skip_space();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_space();
            items.push(self.value()?);
            self.skip_space();
            match self.peek() {
                Some(b',') => {
                    self.at += 1;
                }
                Some(b']') => {
                    self.at += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(format!("expected ',' or ']' at byte {}", self.at)),
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.at += 1; // opening quote
        let mut out = String::new();
        loop {
            let start = self.at;
            while self.at < self.bytes.len() && !matches!(self.bytes[self.at], b'"' | b'\\') && self.bytes[self.at] >= 0x20 {
                self.at += 1;
            }
            if self.at > start {
                let run = std::str::from_utf8(&self.bytes[start..self.at]).map_err(|_| "invalid UTF-8 in a string".to_owned())?;
                out.push_str(run);
            }
            match self.peek() {
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.at += 1;
                    out.push(self.escape()?);
                }
                Some(_) => return Err(format!("control character in a string at byte {}", self.at)),
                None => return Err("unterminated string".into()),
            }
        }
    }

    fn escape(&mut self) -> Result<char, String> {
        let c = self.peek().ok_or_else(|| "unterminated escape".to_owned())?;
        self.at += 1;
        match c {
            b'"' => Ok('"'),
            b'\\' => Ok('\\'),
            b'/' => Ok('/'),
            b'b' => Ok('\u{08}'),
            b'f' => Ok('\u{0c}'),
            b'n' => Ok('\n'),
            b'r' => Ok('\r'),
            b't' => Ok('\t'),
            b'u' => {
                let first = self.hex4()?;
                let code = if (0xd800..=0xdbff).contains(&first) {
                    if self.peek() != Some(b'\\') {
                        return Err("a high surrogate must be followed by a low surrogate".into());
                    }
                    self.at += 1;
                    if self.peek() != Some(b'u') {
                        return Err("a high surrogate must be followed by a low surrogate".into());
                    }
                    self.at += 1;
                    let second = self.hex4()?;
                    if !(0xdc00..=0xdfff).contains(&second) {
                        return Err("invalid low surrogate".into());
                    }
                    0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00)
                } else if (0xdc00..=0xdfff).contains(&first) {
                    return Err("a lone low surrogate is not allowed".into());
                } else {
                    first
                };
                char::from_u32(code).ok_or_else(|| "invalid Unicode escape".to_owned())
            }
            _ => Err(format!("unknown escape \\{}", c as char)),
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        if self.at + 4 > self.bytes.len() {
            return Err("short Unicode escape".into());
        }
        let mut n = 0u32;
        for _ in 0..4 {
            let c = self.bytes[self.at];
            self.at += 1;
            n = n * 16
                + match c {
                    b'0'..=b'9' => u32::from(c - b'0'),
                    b'a'..=b'f' => u32::from(c - b'a') + 10,
                    b'A'..=b'F' => u32::from(c - b'A') + 10,
                    _ => return Err("invalid hex digit in a Unicode escape".into()),
                };
        }
        Ok(n)
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        match self.peek() {
            Some(b'0') => self.at += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.at += 1;
                }
            }
            _ => return Err(format!("invalid number at byte {start}")),
        }
        if self.peek() == Some(b'.') {
            self.at += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(format!("invalid number at byte {start}"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.at += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(format!("invalid number at byte {start}"));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.at += 1;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.at]).map_err(|_| "invalid number".to_owned())?;
        let n: f64 = text.parse().map_err(|_| format!("invalid number {text:?}"))?;
        if !n.is_finite() {
            return Err("a number must be finite".into());
        }
        Ok(Json::Number(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_objects_arrays_and_scalars() {
        let v = parse(r#"{"a":[1,true,null,"x"],"b":{"c":-2.5e2}}"#).unwrap();
        assert_eq!(v.get("a").and_then(Json::as_array).map(<[Json]>::len), Some(4));
        assert_eq!(v.get("a").and_then(|a| a.as_array()).and_then(|a| a[3].as_str()), Some("x"));
        assert_eq!(v.get("b").and_then(|b| b.get("c")).and_then(Json::as_u64), None);
        assert_eq!(v.get("b").and_then(|b| b.get("c")), Some(&Json::Number(-250.0)));
    }

    #[test]
    fn strings_escape_and_unicode_round_trip() {
        let text = "quote \" slash \\ line\n tab\t snowman \u{2603} emoji \u{1f600}";
        let encoded = stringify(&string(text));
        assert_eq!(parse(&encoded).unwrap(), string(text));
        assert_eq!(parse(r#""\u2603 \ud83d\ude00""#).unwrap(), string("\u{2603} \u{1f600}"));
        assert_eq!(parse(r#""\b\f\n\r\t""#).unwrap(), string("\u{8}\u{c}\n\r\t"));
    }

    #[test]
    fn rejects_bad_json() {
        for bad in [
            "",
            "{",
            "{\"a\":}",
            "[1,]",
            "{\"a\":1,}",
            "\"unterminated",
            "\"\\q\"",
            "\"\\ud800\"",
            "\"\\udc00\"",
            "\"\\ud800\\u0041\"",
            "01",
            "1e",
            "true false",
            "NaN",
        ] {
            assert!(parse(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn numbers_are_whole_only_when_they_are_whole() {
        assert_eq!(parse("3").unwrap().as_u64(), Some(3));
        assert_eq!(parse("3.0").unwrap().as_u64(), Some(3));
        assert_eq!(parse("3.5").unwrap().as_u64(), None);
        assert_eq!(parse("-3").unwrap().as_u64(), None);
        assert_eq!(parse("1e3").unwrap().as_u64(), Some(1000));
    }

    #[test]
    fn objects_keep_field_order() {
        let v = parse(r#"{"b":1,"a":2}"#).unwrap();
        assert_eq!(stringify(&v), r#"{"b":1,"a":2}"#);
    }
}
