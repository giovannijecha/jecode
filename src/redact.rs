use crate::json::Value;

// No Debug implementation: this value owns a credential.
#[derive(Clone)]
pub struct Redactor {
    secrets: Vec<(String, String)>,
}

impl Redactor {
    pub fn empty() -> Self {
        Self { secrets: vec![] }
    }

    pub fn byte_prefix(&self, bytes: &[u8], finish: bool) -> (usize, Vec<u8>) {
        let patterns: Vec<_> = self
            .secrets
            .iter()
            .flat_map(|(raw, escaped)| [raw.as_bytes(), escaped.as_bytes()])
            .filter(|pattern| !pattern.is_empty())
            .collect();
        let hold = patterns
            .iter()
            .map(|pattern| pattern.len().saturating_sub(1))
            .max()
            .unwrap_or(0);
        let safe = if finish {
            bytes.len()
        } else {
            bytes.len().saturating_sub(hold)
        };
        let mut consumed = 0;
        let mut result = Vec::new();
        while consumed < safe {
            if let Some(pattern) = patterns
                .iter()
                .find(|pattern| bytes[consumed..].starts_with(pattern))
            {
                result.extend_from_slice(b"[redacted]");
                consumed += pattern.len();
            } else {
                result.push(bytes[consumed]);
                consumed += 1;
            }
        }
        (consumed, result)
    }

    pub fn new(secret: String) -> Self {
        let encoded = Value::string(&secret).encode();
        Self {
            secrets: vec![(secret, encoded[1..encoded.len() - 1].into())],
        }
    }

    pub fn text(&self, text: &str) -> String {
        let mut text = text.to_string();
        for (secret, escaped) in &self.secrets {
            text = text.replace(secret, "[redacted]");
            if escaped != secret {
                text = text.replace(escaped, "[redacted]");
            }
        }
        text
    }

    pub fn include(&mut self, other: Self) {
        for entry in other.secrets {
            if !self.secrets.contains(&entry) {
                self.secrets.push(entry);
            }
        }
        self.secrets
            .sort_by_key(|entry| std::cmp::Reverse(entry.0.len()));
    }

    pub fn text_cursor(&self, text: &str, cursor: usize) -> (String, usize) {
        let mut text = text.to_string();
        let mut cursor = cursor;
        for (secret, escaped) in &self.secrets {
            for pattern in [secret, escaped] {
                let matches: Vec<_> = text
                    .match_indices(pattern)
                    .map(|(start, _)| start)
                    .collect();
                for start in matches.into_iter().rev() {
                    let end = start + pattern.len();
                    if cursor >= end {
                        cursor = cursor - pattern.len() + "[redacted]".len();
                    } else if cursor > start {
                        cursor = start + "[redacted]".len();
                    }
                    text.replace_range(start..end, "[redacted]");
                }
            }
        }
        (text, cursor)
    }

    pub fn value(&self, value: &Value) -> Value {
        match value {
            Value::String(text) => Value::string(self.text(text)),
            Value::Array(values) => {
                Value::Array(values.iter().map(|value| self.value(value)).collect())
            }
            Value::Object(entries) => Value::Object(
                entries
                    .iter()
                    .map(|(key, value)| (self.text(key), self.value(value)))
                    .collect(),
            ),
            value => value.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;

    #[test]
    fn masks_nested_and_json_encoded_credentials_without_changing_other_data() {
        let key = "fixture-quote\"and\\slash";
        let payload = Value::object([
            ("stdout", Value::string(key)),
            ("exit_code", Value::number(0)),
        ]);
        let message = Value::object([("content", Value::string(payload.encode()))]);
        let redacted = Redactor::new(key.into()).value(&message);
        let decoded =
            json::parse(redacted.get("content").and_then(Value::as_str).unwrap()).unwrap();
        assert_eq!(
            decoded.get("stdout").and_then(Value::as_str),
            Some("[redacted]")
        );
        assert_eq!(decoded.get("exit_code"), Some(&Value::number(0)));
    }
}
