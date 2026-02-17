/// Minimal recursive-descent JSON parser for TextMate grammar and theme files.
/// Zero external dependencies — operates on byte slices for efficiency.

#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    pub fn parse(input: &str) -> Result<JsonValue, String> {
        let mut parser = Parser {
            input: input.as_bytes(),
            pos: 0,
        };
        parser.skip_whitespace();
        let value = parser.parse_value()?;
        parser.skip_whitespace();
        if parser.pos < parser.input.len() {
            return Err(format!(
                "JSON error at byte {}: unexpected trailing content",
                parser.pos
            ));
        }
        Ok(value)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            JsonValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            JsonValue::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            JsonValue::Array(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, JsonValue)]> {
        match self {
            JsonValue::Object(o) => Some(o),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            JsonValue::Object(pairs) => {
                for (k, v) in pairs {
                    if k == key {
                        return Some(v);
                    }
                }
                None
            }
            _ => None,
        }
    }
}

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let b = self.input.get(self.pos).copied()?;
        self.pos += 1;
        Some(b)
    }

    fn expect(&mut self, expected: u8) -> Result<(), String> {
        match self.advance() {
            Some(b) if b == expected => Ok(()),
            Some(b) => Err(format!(
                "JSON error at byte {}: expected '{}', found '{}'",
                self.pos - 1,
                expected as char,
                b as char
            )),
            None => Err(format!(
                "JSON error at byte {}: expected '{}', found EOF",
                self.pos, expected as char
            )),
        }
    }

    fn skip_whitespace(&mut self) {
        while let Some(b) = self.peek() {
            match b {
                b' ' | b'\t' | b'\n' | b'\r' => {
                    self.pos += 1;
                }
                _ => break,
            }
        }
    }

    fn parse_value(&mut self) -> Result<JsonValue, String> {
        match self.peek() {
            Some(b'"') => self.parse_string().map(JsonValue::String),
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b't') | Some(b'f') => self.parse_bool(),
            Some(b'n') => self.parse_null(),
            Some(b'-') | Some(b'0'..=b'9') => self.parse_number(),
            Some(b) => Err(format!(
                "JSON error at byte {}: unexpected character '{}'",
                self.pos, b as char
            )),
            None => Err(format!("JSON error at byte {}: unexpected EOF", self.pos)),
        }
    }

    fn parse_null(&mut self) -> Result<JsonValue, String> {
        let start = self.pos;
        for expected in b"null" {
            match self.advance() {
                Some(b) if b == *expected => {}
                _ => return Err(format!("JSON error at byte {}: invalid token", start)),
            }
        }
        Ok(JsonValue::Null)
    }

    fn parse_bool(&mut self) -> Result<JsonValue, String> {
        let start = self.pos;
        if self.peek() == Some(b't') {
            for expected in b"true" {
                match self.advance() {
                    Some(b) if b == *expected => {}
                    _ => return Err(format!("JSON error at byte {}: invalid token", start)),
                }
            }
            Ok(JsonValue::Bool(true))
        } else {
            for expected in b"false" {
                match self.advance() {
                    Some(b) if b == *expected => {}
                    _ => return Err(format!("JSON error at byte {}: invalid token", start)),
                }
            }
            Ok(JsonValue::Bool(false))
        }
    }

    fn parse_number(&mut self) -> Result<JsonValue, String> {
        let start = self.pos;
        // Optional minus
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        // Integer part
        match self.peek() {
            Some(b'0') => {
                self.pos += 1;
            }
            Some(b'1'..=b'9') => {
                self.pos += 1;
                while let Some(b'0'..=b'9') = self.peek() {
                    self.pos += 1;
                }
            }
            _ => return Err(format!("JSON error at byte {}: expected digit", self.pos)),
        }
        // Fractional part
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(format!(
                    "JSON error at byte {}: expected digit after decimal point",
                    self.pos
                ));
            }
            while let Some(b'0'..=b'9') = self.peek() {
                self.pos += 1;
            }
        }
        // Exponent part
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(format!(
                    "JSON error at byte {}: expected digit in exponent",
                    self.pos
                ));
            }
            while let Some(b'0'..=b'9') = self.peek() {
                self.pos += 1;
            }
        }
        let s = std::str::from_utf8(&self.input[start..self.pos])
            .map_err(|_| format!("JSON error at byte {}: invalid UTF-8 in number", start))?;
        let n: f64 = s
            .parse()
            .map_err(|_| format!("JSON error at byte {}: invalid number '{}'", start, s))?;
        Ok(JsonValue::Number(n))
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut result = String::new();
        loop {
            match self.advance() {
                Some(b'"') => return Ok(result),
                Some(b'\\') => {
                    let ch = self.parse_escape()?;
                    result.push(ch);
                }
                Some(b) if b >= 0x80 => {
                    // UTF-8 multi-byte: rewind and decode
                    self.pos -= 1;
                    let ch = self.decode_utf8_char()?;
                    result.push(ch);
                }
                Some(b) => result.push(b as char),
                None => {
                    return Err(format!(
                        "JSON error at byte {}: unterminated string",
                        self.pos
                    ));
                }
            }
        }
    }

    fn parse_escape(&mut self) -> Result<char, String> {
        match self.advance() {
            Some(b'"') => Ok('"'),
            Some(b'\\') => Ok('\\'),
            Some(b'/') => Ok('/'),
            Some(b'n') => Ok('\n'),
            Some(b't') => Ok('\t'),
            Some(b'r') => Ok('\r'),
            Some(b'b') => Ok('\u{0008}'),
            Some(b'f') => Ok('\u{000C}'),
            Some(b'u') => self.parse_unicode_escape(),
            Some(b) => Err(format!(
                "JSON error at byte {}: invalid escape '\\{}'",
                self.pos - 1,
                b as char
            )),
            None => Err(format!(
                "JSON error at byte {}: unexpected EOF in escape",
                self.pos
            )),
        }
    }

    fn parse_unicode_escape(&mut self) -> Result<char, String> {
        let code = self.parse_hex4()?;
        // Check for surrogate pair
        if (0xD800..=0xDBFF).contains(&code) {
            // High surrogate — expect \uXXXX low surrogate
            let pos = self.pos;
            if self.advance() == Some(b'\\') && self.advance() == Some(b'u') {
                let low = self.parse_hex4()?;
                if (0xDC00..=0xDFFF).contains(&low) {
                    let cp = 0x10000 + ((code as u32 - 0xD800) << 10) + (low as u32 - 0xDC00);
                    return char::from_u32(cp).ok_or_else(|| {
                        format!("JSON error at byte {}: invalid surrogate pair", pos)
                    });
                }
            }
            Err(format!(
                "JSON error at byte {}: expected low surrogate after high surrogate",
                pos
            ))
        } else if (0xDC00..=0xDFFF).contains(&code) {
            Err(format!(
                "JSON error at byte {}: unexpected low surrogate",
                self.pos
            ))
        } else {
            char::from_u32(code as u32).ok_or_else(|| {
                format!("JSON error at byte {}: invalid unicode codepoint", self.pos)
            })
        }
    }

    fn parse_hex4(&mut self) -> Result<u16, String> {
        let start = self.pos;
        let mut val: u16 = 0;
        for _ in 0..4 {
            let b = self.advance().ok_or_else(|| {
                format!(
                    "JSON error at byte {}: unexpected EOF in unicode escape",
                    self.pos
                )
            })?;
            let digit = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => {
                    return Err(format!(
                        "JSON error at byte {}: invalid hex digit '{}'",
                        self.pos - 1,
                        b as char
                    ));
                }
            };
            val = val * 16 + digit as u16;
        }
        let _ = start;
        Ok(val)
    }

    fn decode_utf8_char(&mut self) -> Result<char, String> {
        let start = self.pos;
        let s = std::str::from_utf8(&self.input[self.pos..])
            .map_err(|_| format!("JSON error at byte {}: invalid UTF-8", start))?;
        let ch = s
            .chars()
            .next()
            .ok_or_else(|| format!("JSON error at byte {}: unexpected EOF", start))?;
        self.pos += ch.len_utf8();
        Ok(ch)
    }

    fn parse_array(&mut self) -> Result<JsonValue, String> {
        self.expect(b'[')?;
        self.skip_whitespace();
        let mut items = Vec::new();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(JsonValue::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.parse_value()?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(JsonValue::Array(items));
                }
                _ => {
                    return Err(format!(
                        "JSON error at byte {}: expected ',' or ']'",
                        self.pos
                    ));
                }
            }
        }
    }

    fn parse_object(&mut self) -> Result<JsonValue, String> {
        self.expect(b'{')?;
        self.skip_whitespace();
        let mut pairs = Vec::new();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(JsonValue::Object(pairs));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(format!(
                    "JSON error at byte {}: expected string key",
                    self.pos
                ));
            }
            let key = self.parse_string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            self.skip_whitespace();
            let value = self.parse_value()?;
            pairs.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(JsonValue::Object(pairs));
                }
                _ => {
                    return Err(format!(
                        "JSON error at byte {}: expected ',' or '}}'",
                        self.pos
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_null() {
        assert_eq!(JsonValue::parse("null").unwrap(), JsonValue::Null);
    }

    #[test]
    fn test_booleans() {
        assert_eq!(JsonValue::parse("true").unwrap(), JsonValue::Bool(true));
        assert_eq!(JsonValue::parse("false").unwrap(), JsonValue::Bool(false));
    }

    #[test]
    fn test_integers() {
        assert_eq!(JsonValue::parse("0").unwrap().as_f64(), Some(0.0));
        assert_eq!(JsonValue::parse("42").unwrap().as_f64(), Some(42.0));
        assert_eq!(JsonValue::parse("-7").unwrap().as_f64(), Some(-7.0));
    }

    #[test]
    fn test_float_and_exponent() {
        assert_eq!(JsonValue::parse("3.14").unwrap().as_f64(), Some(3.14));
        assert_eq!(JsonValue::parse("1e10").unwrap().as_f64(), Some(1e10));
        assert_eq!(JsonValue::parse("-2.5E-3").unwrap().as_f64(), Some(-2.5e-3));
    }

    #[test]
    fn test_simple_string() {
        assert_eq!(
            JsonValue::parse(r#""hello""#).unwrap().as_str(),
            Some("hello")
        );
    }

    #[test]
    fn test_string_escapes() {
        let val = JsonValue::parse(r#""a\nb\t\\\"\/""#).unwrap();
        assert_eq!(val.as_str(), Some("a\nb\t\\\"/"));
    }

    #[test]
    fn test_unicode_escape() {
        let val = JsonValue::parse(r#""\u0041\u00e9""#).unwrap();
        assert_eq!(val.as_str(), Some("A\u{00e9}"));
    }

    #[test]
    fn test_surrogate_pair() {
        // U+1F600 = D83D DE00
        let val = JsonValue::parse(r#""\uD83D\uDE00""#).unwrap();
        assert_eq!(val.as_str(), Some("\u{1F600}"));
    }

    #[test]
    fn test_empty_array() {
        let val = JsonValue::parse("[]").unwrap();
        assert_eq!(val.as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_nested_array() {
        let val = JsonValue::parse("[1, [2, 3], []]").unwrap();
        let arr = val.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[1].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_empty_object() {
        let val = JsonValue::parse("{}").unwrap();
        assert_eq!(val.as_object().unwrap().len(), 0);
    }

    #[test]
    fn test_nested_object() {
        let val = JsonValue::parse(r#"{"a": {"b": 1}, "c": 2}"#).unwrap();
        assert_eq!(val.get("a").unwrap().get("b").unwrap().as_f64(), Some(1.0));
        assert_eq!(val.get("c").unwrap().as_f64(), Some(2.0));
    }

    #[test]
    fn test_object_order_preserved() {
        let val = JsonValue::parse(r#"{"z": 1, "a": 2, "m": 3}"#).unwrap();
        let pairs = val.as_object().unwrap();
        assert_eq!(pairs[0].0, "z");
        assert_eq!(pairs[1].0, "a");
        assert_eq!(pairs[2].0, "m");
    }

    #[test]
    fn test_accessor_miss() {
        let val = JsonValue::parse("42").unwrap();
        assert_eq!(val.as_str(), None);
        assert_eq!(val.as_bool(), None);
        assert_eq!(val.as_array(), None);
        assert_eq!(val.as_object(), None);
        assert_eq!(val.get("x"), None);
    }

    #[test]
    fn test_error_unexpected_eof() {
        assert!(JsonValue::parse("").is_err());
        assert!(JsonValue::parse("[1,").is_err());
    }

    #[test]
    fn test_error_invalid_token() {
        assert!(JsonValue::parse("tru").is_err());
        assert!(JsonValue::parse("nulx").is_err());
    }

    #[test]
    fn test_error_trailing_content() {
        assert!(JsonValue::parse("1 2").is_err());
    }

    #[test]
    fn test_textmate_grammar_snippet() {
        let json = r#"{
            "name": "source.example",
            "scopeName": "source.example",
            "patterns": [
                {
                    "match": "\\b(if|else|while)\\b",
                    "name": "keyword.control"
                },
                {
                    "begin": "\"",
                    "end": "\"",
                    "name": "string.quoted.double",
                    "patterns": [
                        {"match": "\\\\.", "name": "constant.character.escape"}
                    ]
                }
            ],
            "repository": {
                "comments": {
                    "match": "//.*$",
                    "name": "comment.line"
                }
            }
        }"#;
        let val = JsonValue::parse(json).unwrap();
        assert_eq!(val.get("name").unwrap().as_str(), Some("source.example"));
        let patterns = val.get("patterns").unwrap().as_array().unwrap();
        assert_eq!(patterns.len(), 2);
        assert_eq!(
            patterns[0].get("name").unwrap().as_str(),
            Some("keyword.control")
        );
        let inner = patterns[1].get("patterns").unwrap().as_array().unwrap();
        assert_eq!(inner.len(), 1);
        let repo = val.get("repository").unwrap();
        assert!(repo.get("comments").is_some());
    }
}
