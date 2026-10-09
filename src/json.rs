use std::collections::BTreeMap;
use std::fmt::Write;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

impl Value {
    pub fn object<const N: usize>(entries: [(&str, Value); N]) -> Self {
        Self::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect(),
        )
    }

    pub fn string(value: impl Into<String>) -> Self {
        Self::String(value.into())
    }

    pub fn number(value: impl ToString) -> Self {
        Self::Number(value.to_string())
    }

    pub fn get(&self, key: &str) -> Option<&Self> {
        match self {
            Self::Object(entries) => entries.get(key),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(values) => Some(values),
            _ => None,
        }
    }

    pub fn as_usize(&self) -> Option<usize> {
        match self {
            Self::Number(value) => value.parse().ok(),
            _ => None,
        }
    }

    pub fn encode(&self) -> String {
        let mut output = String::new();
        self.write(&mut output);
        output
    }

    pub fn pretty(&self) -> String {
        fn render(value: &Value, depth: usize) -> String {
            let (open, close, entries): (char, char, Vec<String>) = match value {
                Value::Object(entries) if !entries.is_empty() => (
                    '{',
                    '}',
                    entries
                        .iter()
                        .map(|(key, value)| {
                            format!(
                                "{}: {}",
                                Value::string(key).encode(),
                                render(value, depth + 1)
                            )
                        })
                        .collect(),
                ),
                Value::Array(values) if !values.is_empty() => (
                    '[',
                    ']',
                    values
                        .iter()
                        .map(|value| render(value, depth + 1))
                        .collect(),
                ),
                _ => return value.encode(),
            };
            let indent = "  ".repeat(depth + 1);
            format!(
                "{open}\n{indent}{}\n{}{close}",
                entries.join(&format!(",\n{indent}")),
                "  ".repeat(depth)
            )
        }
        render(self, 0)
    }

    fn write(&self, output: &mut String) {
        match self {
            Self::Null => output.push_str("null"),
            Self::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
            Self::Number(value) => output.push_str(value),
            Self::String(value) => {
                output.push('"');
                for character in value.chars() {
                    match character {
                        '"' => output.push_str("\\\""),
                        '\\' => output.push_str("\\\\"),
                        '\n' => output.push_str("\\n"),
                        '\r' => output.push_str("\\r"),
                        '\t' => output.push_str("\\t"),
                        character if character <= '\u{1f}' => {
                            write!(output, "\\u{:04x}", character as u32)
                                .expect("writing to a string");
                        }
                        character => output.push(character),
                    }
                }
                output.push('"');
            }
            Self::Array(values) => {
                output.push('[');
                for (index, value) in values.iter().enumerate() {
                    if index != 0 {
                        output.push(',');
                    }
                    value.write(output);
                }
                output.push(']');
            }
            Self::Object(entries) => {
                output.push('{');
                for (index, (key, value)) in entries.iter().enumerate() {
                    if index != 0 {
                        output.push(',');
                    }
                    Self::String(key.clone()).write(output);
                    output.push(':');
                    value.write(output);
                }
                output.push('}');
            }
        }
    }
}

pub fn parse(input: &str) -> Result<Value, String> {
    let mut parser = Parser { input, position: 0 };
    let value = parser.value(0)?;
    parser.whitespace();
    if parser.position != input.len() {
        return Err(parser.error("unexpected trailing data"));
    }
    Ok(value)
}

struct Parser<'a> {
    input: &'a str,
    position: usize,
}

impl Parser<'_> {
    fn error(&self, message: &str) -> String {
        format!("Invalid JSON at byte {}: {message}", self.position)
    }

    fn peek(&self) -> Option<u8> {
        self.input.as_bytes().get(self.position).copied()
    }

    fn whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.position += 1;
        }
    }

    fn take(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > 64 {
            return Err(self.error("nesting limit exceeded"));
        }
        self.whitespace();
        match self.peek() {
            Some(b'n') => self.literal("null", Value::Null),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'"') => self.string().map(Value::String),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(b'[') => {
                self.position += 1;
                self.whitespace();
                let mut values = Vec::new();
                if !self.take(b']') {
                    loop {
                        values.push(self.value(depth + 1)?);
                        self.whitespace();
                        if self.take(b']') {
                            break;
                        }
                        if !self.take(b',') {
                            return Err(self.error("expected comma or closing bracket"));
                        }
                    }
                }
                Ok(Value::Array(values))
            }
            Some(b'{') => {
                self.position += 1;
                self.whitespace();
                let mut entries = BTreeMap::new();
                if !self.take(b'}') {
                    loop {
                        self.whitespace();
                        let key = self.string()?;
                        self.whitespace();
                        if !self.take(b':') {
                            return Err(self.error("expected colon"));
                        }
                        let value = self.value(depth + 1)?;
                        if entries.insert(key, value).is_some() {
                            return Err(self.error("duplicate object key"));
                        }
                        self.whitespace();
                        if self.take(b'}') {
                            break;
                        }
                        if !self.take(b',') {
                            return Err(self.error("expected comma or closing brace"));
                        }
                    }
                }
                Ok(Value::Object(entries))
            }
            _ => Err(self.error("expected a value")),
        }
    }

    fn literal(&mut self, text: &str, value: Value) -> Result<Value, String> {
        if !self.input[self.position..].starts_with(text) {
            return Err(self.error("invalid literal"));
        }
        self.position += text.len();
        Ok(value)
    }

    fn number(&mut self) -> Result<Value, String> {
        let start = self.position;
        self.take(b'-');
        if !self.take(b'0') {
            if !matches!(self.peek(), Some(b'1'..=b'9')) {
                return Err(self.error("expected a digit"));
            }
            self.digits();
        }
        if self.take(b'.') && !self.digits() {
            return Err(self.error("expected fractional digits"));
        }
        if self.take(b'e') || self.take(b'E') {
            if !self.take(b'+') {
                self.take(b'-');
            }
            if !self.digits() {
                return Err(self.error("expected exponent digits"));
            }
        }
        Ok(Value::Number(self.input[start..self.position].into()))
    }

    fn digits(&mut self) -> bool {
        let start = self.position;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.position += 1;
        }
        self.position > start
    }

    fn string(&mut self) -> Result<String, String> {
        if !self.take(b'"') {
            return Err(self.error("expected a string"));
        }
        let mut output = String::new();
        loop {
            match self.peek() {
                Some(b'"') => {
                    self.position += 1;
                    return Ok(output);
                }
                Some(b'\\') => {
                    self.position += 1;
                    let escape = self.peek().ok_or_else(|| self.error("unfinished escape"))?;
                    self.position += 1;
                    output.push(match escape {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{08}',
                        b'f' => '\u{0c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.unicode()?,
                        _ => return Err(self.error("invalid string escape")),
                    });
                }
                Some(0..=31) => return Err(self.error("unescaped control character")),
                Some(_) => {
                    let character = self.input[self.position..].chars().next().unwrap();
                    output.push(character);
                    self.position += character.len_utf8();
                }
                None => return Err(self.error("unterminated string")),
            }
        }
    }

    fn unicode(&mut self) -> Result<char, String> {
        let first = self.hex_quad()?;
        let code = if (0xd800..=0xdbff).contains(&first) {
            if !self.take(b'\\') || !self.take(b'u') {
                return Err(self.error("missing low surrogate"));
            }
            let second = self.hex_quad()?;
            if !(0xdc00..=0xdfff).contains(&second) {
                return Err(self.error("invalid low surrogate"));
            }
            0x10000 + ((first - 0xd800) << 10) + second - 0xdc00
        } else {
            first
        };
        char::from_u32(code).ok_or_else(|| self.error("invalid Unicode scalar"))
    }

    fn hex_quad(&mut self) -> Result<u32, String> {
        let mut value = 0;
        for _ in 0..4 {
            let digit = match self.peek() {
                Some(byte @ b'0'..=b'9') => u32::from(byte - b'0'),
                Some(byte @ b'a'..=b'f') => u32::from(byte - b'a' + 10),
                Some(byte @ b'A'..=b'F') => u32::from(byte - b'A' + 10),
                _ => return Err(self.error("expected four hexadecimal digits")),
            };
            self.position += 1;
            value = (value << 4) | digit;
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_tool_arguments_and_unicode() {
        let value = Value::object([
            ("command", Value::string("printf 'héllo 🦀'\n\t\"\\\u{0}")),
            (
                "values",
                Value::Array(vec![Value::Null, Value::Bool(true), Value::number(123)]),
            ),
        ]);
        assert_eq!(parse(&value.encode()).unwrap(), value);
        assert_eq!(parse(r#""\ud83e\udd80""#).unwrap().as_str(), Some("🦀"));
    }

    #[test]
    fn numbers_are_preserved_without_rounding() {
        let text = "[9007199254740993,-0,1.25e+100]";
        assert_eq!(parse(text).unwrap().encode(), text);
    }

    #[test]
    fn rejects_malformed_and_ambiguous_json() {
        for text in [
            "",
            "01",
            "-",
            "1.",
            "1e+",
            "NaN",
            "true false",
            "[1,]",
            "{\"a\":1,}",
            "{\"a\":1,\"a\":2}",
            "\"\n\"",
            r#""\x""#,
            r#""\ud800""#,
            r#""\udc00""#,
            r#""\ud800\u0041""#,
        ] {
            assert!(parse(text).is_err(), "accepted {text:?}");
        }
        assert!(parse(&format!("{}0{}", "[".repeat(100), "]".repeat(100))).is_err());
    }
}
