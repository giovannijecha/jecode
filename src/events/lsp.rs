use crate::json::Value;

pub(super) fn summary(result: &Value) -> String {
    let text = |key| result.get(key).and_then(Value::as_str).unwrap_or("");
    let status = text("status");
    let detail = text("detail");
    if !text("error").is_empty() {
        return format!("error: {}", text("error"));
    }
    if matches!(
        status,
        "failed"
            | "configuration_error"
            | "invalid_input"
            | "server_error"
            | "crashed"
            | "not_found"
            | "restart_suppressed"
    ) {
        return format!("error: {detail}");
    }
    if matches!(status, "cancelled" | "timed_out" | "settings_changed") {
        return format!("stopped: {detail}");
    }
    let mut summary = if text("action") == "diagnostics" {
        let freshness = text("freshness");
        let diagnostics = result
            .get("diagnostics")
            .and_then(Value::as_array)
            .unwrap_or(&[]);
        if diagnostics.is_empty() && freshness != "current" {
            format!("Diagnostics {freshness}; this document only")
        } else {
            let errors = diagnostics
                .iter()
                .filter(|item| item.get("severity").and_then(Value::as_usize) == Some(1))
                .count();
            let warnings = diagnostics
                .iter()
                .filter(|item| item.get("severity").and_then(Value::as_usize) == Some(2))
                .count();
            format!("{errors} errors, {warnings} warnings; {freshness}; this document only")
        }
    } else {
        detail.to_owned()
    };
    if result.get("complete") == Some(&Value::Bool(false)) {
        summary.push_str("; incomplete");
    }
    if result.get("truncated") == Some(&Value::Bool(true)) {
        summary.push_str(if result.get("result_file").is_some() {
            "; preview, response retained"
        } else {
            "; result collection truncated"
        });
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_pending_is_not_reported_as_a_current_empty_diagnostic_set() {
        let pending = Value::object([
            ("action", Value::string("diagnostics")),
            ("status", Value::string("ready")),
            ("freshness", Value::string("pending")),
            ("diagnostics", Value::Array(vec![])),
        ]);
        assert_eq!(
            crate::events::tool_summary("lsp", &pending),
            "Diagnostics pending; this document only"
        );
        let mut current = pending;
        if let Value::Object(fields) = &mut current {
            fields.insert("freshness".into(), Value::string("current"));
        }
        assert_eq!(
            crate::events::tool_summary("lsp", &current),
            "0 errors, 0 warnings; current; this document only"
        );
    }

    #[test]
    fn cli_uses_the_saved_display_name_and_preserves_cancelled_outcome() {
        let arguments = Value::object([
            ("server", Value::string("stable-id")),
            ("server_name", Value::string("Rust")),
            ("action", Value::string("definition")),
        ]);
        assert_eq!(
            crate::events::target("lsp", &arguments),
            "Rust › definition"
        );
        let result = Value::object([
            ("status", Value::string("cancelled")),
            (
                "detail",
                Value::string("Request cancelled; incomplete results"),
            ),
        ]);
        assert_eq!(
            crate::events::tool_summary("lsp", &result),
            "stopped: Request cancelled; incomplete results"
        );
    }
}
