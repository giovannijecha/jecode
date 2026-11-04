//! A call that names several files, told as one line.
use crate::json::Value;

/// Whether an edit result changed its file. New receipts say so explicitly;
/// older ones retain the bytes/checkpoint fallback, which cannot distinguish
/// an empty replacement without a saved checkpoint from a no-op.
pub fn file_changed(result: &Value) -> bool {
    let checkpoint = result
        .get("checkpoint")
        .and_then(|checkpoint| checkpoint.get("status"))
        .and_then(Value::as_str);
    result.get("error").is_none()
        && match result.get("changed") {
            Some(Value::Bool(changed)) => *changed,
            _ => {
                result.get("bytes_written").and_then(Value::as_usize) > Some(0)
                    || matches!(checkpoint, Some("saved" | "partial"))
            }
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

/// Each member's checkpoint problem once, naming the files it reached.
pub(super) fn warnings(result: &Value) -> Vec<String> {
    let mut warnings: Vec<(String, Vec<&str>)> = Vec::new();
    for file in result.get("files").and_then(Value::as_array).unwrap_or(&[]) {
        let Some(warning) = super::checkpoint_warning(file) else {
            continue;
        };
        let path = file.get("path").and_then(Value::as_str).unwrap_or("");
        match warnings.iter_mut().find(|(seen, _)| *seen == warning) {
            Some((_, paths)) => paths.push(path),
            None => warnings.push((warning, vec![path])),
        }
    }
    warnings
        .into_iter()
        .map(|(warning, paths)| format!("{warning} ({})", paths.join(", ")))
        .collect()
}
