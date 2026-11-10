//! A call that names several files, told as one line.
use crate::json::Value;

/// Whether an edit result changed its file. New receipts say so explicitly;
/// older ones fall back to the bytes written.
pub fn file_changed(result: &Value) -> bool {
    result.get("error").is_none()
        && match result.get("changed") {
            Some(Value::Bool(changed)) => *changed,
            _ => result.get("bytes_written").and_then(Value::as_usize) > Some(0),
        }
}

/// `changed 2 files; 1 failed`, `no files read; 2 not read`.
pub(super) fn summary(name: &str, files: &[Value]) -> Option<String> {
    let missed = files
        .iter()
        .filter(|file| file.get("error").is_some())
        .count();
    let (done, verb, missing) = match name {
        "edit" => (
            files.iter().filter(|file| file_changed(file)).count(),
            "changed",
            "failed",
        ),
        "read" => (files.len() - missed, "read", "not read"),
        _ => return None,
    };
    let mut summary = match done {
        0 => format!("no files {verb}"),
        done => format!("{verb} {}", super::counted(done, "file")),
    };
    if missed > 0 {
        summary.push_str(&format!("; {missed} {missing}"));
    }
    Some(summary)
}
