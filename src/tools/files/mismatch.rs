use crate::json::Value;

/// A rejected edit supplies bounded literal context, never a fuzzy replacement.
pub(super) fn result(content: &str, error: &str) -> Value {
    const LIMIT: usize = 8192;
    let mut end = content.len().min(LIMIT);
    if content.len() > LIMIT {
        end = LIMIT / 2;
    }
    while !content.is_char_boundary(end) {
        end -= 1;
    }
    let mut start = content.len().saturating_sub(LIMIT / 2);
    while !content.is_char_boundary(start) {
        start += 1;
    }
    let mut result = Value::object([
        ("error", Value::string(error)),
        ("outcome", Value::string("not_started")),
        ("current_bytes", Value::number(content.len())),
        ("current_content", Value::string(&content[..end])),
        ("context_complete", Value::Bool(end == content.len())),
        (
            "context_notice",
            Value::string(
                "Literal current file text, without line-number prefixes. No replacement was applied. Use exact text; if the required section is absent, read the file before retrying.",
            ),
        ),
    ]);
    if end < content.len()
        && let Value::Object(fields) = &mut result
    {
        fields.insert("current_tail".into(), Value::string(&content[start..]));
        fields.insert("current_tail_byte_offset".into(), Value::number(start));
    }
    result
}
