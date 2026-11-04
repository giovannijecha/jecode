//! A call that names several files, told as one line.
use crate::json::Value;

/// Whether an edit result changed its file. Emptying a file writes no bytes,
/// as a replacement that changes nothing reports too, but only a real write
/// saves a checkpoint first.
pub fn file_changed(result: &Value) -> bool {
    let checkpoint = result
        .get("checkpoint")
        .and_then(|checkpoint| checkpoint.get("status"))
        .and_then(Value::as_str);
    result.get("error").is_none()
        && (result.get("bytes_written").and_then(Value::as_usize) > Some(0)
            || matches!(checkpoint, Some("saved" | "partial")))
}
