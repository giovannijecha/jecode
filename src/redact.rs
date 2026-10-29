use crate::json::Value;
use std::sync::{Arc, Mutex};

type Secrets = Arc<Mutex<Vec<(String, String)>>>;

// No Debug implementation: this value owns a credential.
#[derive(Clone)]
pub struct Redactor {
    sources: Vec<Secrets>,
}

impl Redactor {
    fn secrets(&self) -> Vec<(String, String)> {
        let mut secrets: Vec<_> = self
            .sources
            .iter()
            .flat_map(|source| source.lock().unwrap().clone())
            .filter(|(raw, _)| !raw.is_empty())
            .collect();
        secrets.sort_by_key(|entry| std::cmp::Reverse(entry.0.len()));
        secrets.dedup();
        secrets
    }

    fn patterns(&self) -> Vec<Vec<u8>> {
        self.secrets()
            .into_iter()
            .flat_map(|(raw, escaped)| [raw.into_bytes(), escaped.into_bytes()])
            .filter(|pattern| !pattern.is_empty())
            .collect()
    }

    pub(crate) fn boundary_overlap(&self) -> Option<usize> {
        self.patterns()
            .iter()
            .map(|pattern| pattern.len())
            .max()
            .map(|length| length - 1)
    }

    /// Masks matches that intersect a byte window. A match that began before
    /// the window is omitted; its replacement belongs to the preceding page.
    pub(crate) fn byte_window(&self, bytes: &[u8], start: usize, end: usize) -> Vec<u8> {
        let patterns = self.patterns();
        let mut result = Vec::new();
        let mut index = 0;
        while index < bytes.len() {
            if let Some(pattern) = patterns
                .iter()
                .find(|pattern| bytes[index..].starts_with(pattern))
            {
                if (start..end).contains(&index) {
                    result.extend_from_slice(b"[redacted]");
                }
                index += pattern.len();
            } else {
                if (start..end).contains(&index) {
                    result.push(bytes[index]);
                }
                index += 1;
            }
        }
        result
    }

    pub fn empty() -> Self {
        Self {
            sources: vec![Arc::new(Mutex::new(vec![]))],
        }
    }

    pub fn byte_prefix(&self, bytes: &[u8], finish: bool) -> (usize, Vec<u8>) {
        let patterns = self.patterns();
        // Hold back only a trailing fragment that could still start a secret,
        // so live readers see every byte that can no longer be masked.
        let hold = if finish {
            0
        } else {
            let longest = patterns.iter().map(|pattern| pattern.len()).max();
            (1..=bytes.len().min(longest.unwrap_or(0).saturating_sub(1)))
                .rev()
                .find(|&length| {
                    let suffix = &bytes[bytes.len() - length..];
                    patterns
                        .iter()
                        .any(|pattern| pattern.len() > length && pattern.starts_with(suffix))
                })
                .unwrap_or(0)
        };
        let safe = bytes.len() - hold;
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
        let redactor = Self::empty();
        redactor.add_secret(secret);
        redactor
    }

    /// Existing clones and consumers also mask tokens added after renewal.
    pub fn add_secret(&self, secret: String) {
        if secret.is_empty() {
            return;
        }
        let encoded = Value::string(&secret).encode();
        let entry = (secret, encoded[1..encoded.len() - 1].into());
        let mut secrets = self.sources[0].lock().unwrap();
        if !secrets.contains(&entry) {
            secrets.push(entry);
        }
    }

    pub fn text(&self, text: &str) -> String {
        mask(text, &self.secrets())
    }

    pub fn include(&mut self, other: Self) {
        for source in other.sources {
            if !self.sources.iter().any(|saved| Arc::ptr_eq(saved, &source)) {
                self.sources.push(source);
            }
        }
    }

    pub fn text_cursor(&self, text: &str, cursor: usize) -> (String, usize) {
        let mut text = text.to_string();
        let mut cursor = cursor;
        for (secret, escaped) in &self.secrets() {
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

    /// Reads the secrets once for the whole tree; large tool results hold
    /// thousands of strings.
    pub fn value(&self, value: &Value) -> Value {
        let secrets = self.secrets();
        if secrets.is_empty() {
            return value.clone();
        }
        mask_value(value, &secrets)
    }
}

fn mask(text: &str, secrets: &[(String, String)]) -> String {
    let mut text = text.to_string();
    for (secret, escaped) in secrets {
        for pattern in [secret, escaped] {
            if text.contains(pattern.as_str()) {
                text = text.replace(pattern.as_str(), "[redacted]");
            }
        }
    }
    text
}

fn mask_value(value: &Value, secrets: &[(String, String)]) -> Value {
    match value {
        Value::String(text) => Value::string(mask(text, secrets)),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| mask_value(value, secrets))
                .collect(),
        ),
        Value::Object(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, value)| (mask(key, secrets), mask_value(value, secrets)))
                .collect(),
        ),
        value => value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;

    #[test]
    fn renewal_is_visible_to_registered_consumers_and_keeps_old_tokens_masked() {
        let account = Redactor::new("fixture-access-old".into());
        let mut agent = Redactor::empty();
        agent.include(account.clone());
        let persisted = agent.clone();
        account.add_secret("fixture-access-new".into());
        account.add_secret("fixture-refresh-token".into());
        assert_eq!(
            persisted.text("fixture-access-old fixture-access-new fixture-refresh-token"),
            "[redacted] [redacted] [redacted]"
        );
        assert_eq!(
            persisted.byte_prefix(b"fixture-access-ne", false),
            (0, vec![])
        );
        assert_eq!(
            Redactor::empty().text("fixture-access-new"),
            "fixture-access-new"
        );
    }

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
