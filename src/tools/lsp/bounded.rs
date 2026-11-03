use crate::{json::Value, output};

/// Protocol completeness and inline truncation are independent observations.
pub(super) fn bound(value: Value, outputs: &output::Store) -> Value {
    if value.encode().len() <= crate::tools::OUTPUT_LIMIT {
        return value;
    }
    let retained = save(outputs, &value);
    let Value::Object(mut fields) = value else {
        return Value::Null;
    };
    // Counts come from the full arrays before either truncation drops findings.
    for key in ["locations", "diagnostics"] {
        if let Some(Value::Array(items)) = fields.get_mut(key) {
            let total = items.len();
            let severity = |level| {
                items
                    .iter()
                    .filter(|item| item.get("severity").and_then(Value::as_usize) == Some(level))
                    .count()
            };
            let severities = (key == "diagnostics").then(|| {
                [
                    ("total_errors", severity(1)),
                    ("total_warnings", severity(2)),
                ]
            });
            items.truncate(40);
            fields.insert(format!("total_{key}"), Value::number(total));
            for (name, count) in severities.into_iter().flatten() {
                fields.insert(name.into(), Value::number(count));
            }
        }
    }
    fields.insert("truncated".into(), Value::Bool(true));
    fields.insert("inline_truncated".into(), Value::Bool(true));
    match retained {
        Ok(reference) => {
            fields.insert("result_file".into(), Value::string(reference));
        }
        Err(error) => {
            fields.insert("result_file_error".into(), Value::string(error));
        }
    }
    let mut result = Value::Object(fields);
    if result.encode().len() > crate::tools::OUTPUT_LIMIT {
        let preview = result.encode();
        let preview = excerpt(&preview, 32 * 1024);
        let mut envelope = Value::object([
            ("preview", Value::string(preview)),
            ("truncated", Value::Bool(true)),
            (
                "detail",
                Value::string(
                    "LSP output exceeded the inline budget; read result_file for the complete response.",
                ),
            ),
        ]);
        if let Value::Object(fields) = &mut envelope {
            for key in [
                "action",
                "status",
                "server",
                "server_name",
                "root",
                "freshness",
                "complete",
                "result_file",
                "result_file_error",
                "total_locations",
                "total_diagnostics",
                "total_errors",
                "total_warnings",
            ] {
                if let Some(value) = result.get(key) {
                    let bounded = match value {
                        Value::String(text) => Value::string(excerpt(text, 2048)),
                        _ => value.clone(),
                    };
                    fields.insert(key.into(), bounded);
                }
            }
        }
        result = envelope;
    }
    result
}

fn excerpt(text: &str, budget: usize) -> String {
    let mut end = text.len().min(budget);
    loop {
        let candidate = &text[..text.floor_char_boundary(end)];
        let encoded = Value::string(candidate).encode().len();
        if encoded <= budget || candidate.is_empty() {
            return candidate.into();
        }
        end = candidate.len() * budget / encoded;
    }
}

fn save(outputs: &output::Store, value: &Value) -> Result<String, String> {
    let mut logs = outputs.create()?;
    logs.stdout
        .write(value.pretty().as_bytes())
        .map_err(|error| format!("Could not retain LSP result: {error}"))?;
    logs.stdout.finish().map_err(|error| error.to_string())?;
    logs.stderr.finish().map_err(|error| error.to_string())?;
    Ok(logs.stdout_ref)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{redact::Redactor, test_support::Directory};

    fn diagnostics(findings: Vec<Value>) -> Value {
        Value::object([
            ("action", Value::string("diagnostics")),
            ("status", Value::string("ready")),
            ("freshness", Value::string("current")),
            ("complete", Value::Bool(true)),
            ("diagnostics", Value::Array(findings)),
        ])
    }

    fn finding(severity: usize, message: usize) -> Value {
        Value::object([
            ("severity", Value::number(severity)),
            ("message", Value::string("x".repeat(message))),
        ])
    }

    fn totals(bounded: &Value) -> [Option<usize>; 3] {
        ["total_diagnostics", "total_errors", "total_warnings"]
            .map(|key| bounded.get(key).and_then(Value::as_usize))
    }

    #[test]
    fn an_oversized_single_diagnostic_keeps_its_counts_in_the_envelope() {
        let directory = Directory::new();
        let outputs = output::Store::new(directory.path().join("outputs"), Redactor::empty());
        let bounded = bound(diagnostics(vec![finding(1, 100 * 1024)]), &outputs);
        assert!(bounded.get("diagnostics").is_none());
        assert!(bounded.get("preview").is_some());
        assert_eq!(totals(&bounded), [Some(1), Some(1), Some(0)]);
        assert_eq!(
            crate::events::tool_summary("lsp", &bounded),
            "1 errors, 0 warnings; current; this document only; preview, response retained"
        );
    }

    #[test]
    fn counts_include_findings_beyond_the_first_forty() {
        let directory = Directory::new();
        let outputs = output::Store::new(directory.path().join("outputs"), Redactor::empty());
        // Severities 1, 2, 2, 3 repeat: 25 errors and 50 warnings in 100.
        let findings = (0..100)
            .map(|index| finding([1, 2, 2, 3][index % 4], 1024))
            .collect();
        let bounded = bound(diagnostics(findings), &outputs);
        assert_eq!(
            bounded
                .get("diagnostics")
                .and_then(Value::as_array)
                .map(<[_]>::len),
            Some(40)
        );
        assert_eq!(totals(&bounded), [Some(100), Some(25), Some(50)]);
        assert_eq!(
            crate::events::tool_summary("lsp", &bounded),
            "25 errors, 50 warnings; current; this document only; preview, response retained"
        );
    }
}
