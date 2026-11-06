use crate::json::Value;

/// A rejected edit supplies bounded literal context, never a fuzzy replacement.
pub(super) fn result(content: &str, error: &str) -> Value {
    bounded(content, error, 8192)
}

pub(super) fn bounded(content: &str, error: &str, limit: usize) -> Value {
    let end = if content.len() > limit {
        content.floor_char_boundary(limit / 2)
    } else {
        content.len()
    };
    let start = content.ceil_char_boundary(content.len().saturating_sub(limit / 2));
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

/// Exposes the first two exact matches in the view used for matching. The
/// original-file preview remains separate for batched edits.
pub(super) fn matches(
    result: &mut Value,
    source: &str,
    length: u64,
    offsets: [u64; 2],
    mut read: impl FnMut(u64, usize) -> Result<Vec<u8>, String>,
) -> Result<(), String> {
    let mut windows = Vec::with_capacity(2);
    for offset in offsets {
        let start = offset.saturating_sub(96);
        let size = length.saturating_sub(start).min(256) as usize;
        let bytes = read(start, size)?;
        if bytes.len() != size {
            return Err("The file changed while preparing edit context".into());
        }
        // A bounded byte window can begin or end inside a UTF-8 character.
        let skip = bytes
            .iter()
            .take_while(|&&byte| byte & 0b1100_0000 == 0b1000_0000)
            .count();
        let visible = &bytes[skip..];
        let end = match std::str::from_utf8(visible) {
            Ok(_) => visible.len(),
            Err(error) => error.valid_up_to(),
        };
        let text = std::str::from_utf8(&visible[..end]).map_err(|error| error.to_string())?;
        windows.push(Value::object([
            ("offset", Value::number(offset)),
            ("excerpt_byte_offset", Value::number(start + skip as u64)),
            ("text", Value::string(text)),
        ]));
    }
    if let Value::Object(fields) = result {
        fields.insert("match_context_source".into(), Value::string(source));
        fields.insert("match_context".into(), Value::Array(windows));
    }
    Ok(())
}
