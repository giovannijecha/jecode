use crate::{agent::Agent, json::Value};
use std::io::Write;

/// Plain inspection has the same read-only boundary as the terminal panel.
pub(super) fn print(agent: &Agent, arguments: &str, output: &mut impl Write) -> Result<(), String> {
    let records = agent.collaboration().snapshot();
    render(&records, arguments.trim(), output)
}

fn render(records: &[Value], arguments: &str, output: &mut impl Write) -> Result<(), String> {
    if arguments.is_empty() {
        if records.is_empty() {
            return writeln!(output, "No open assignments.").map_err(|e| e.to_string());
        }
        for record in records {
            writeln!(
                output,
                "AGENT {}  {}  {}",
                number(record),
                text(record, "title"),
                state(record)
            )
            .map_err(|e| e.to_string())?;
        }
        return writeln!(
            output,
            "Use /agents <number> to inspect activity, assignment and result."
        )
        .map_err(|e| e.to_string());
    }
    let id = arguments
        .strip_prefix('a')
        .unwrap_or(arguments)
        .parse::<usize>()
        .map_err(|_| "Usage: /agents [number]".to_owned())?;
    let record = records
        .iter()
        .find(|r| number(r) == id)
        .ok_or("Agent not found among open assignments")?;
    writeln!(
        output,
        "AGENT {id} — {}\n{}\nWorkspace: {}\nDirectory: {}\nAssignment v{}:\n{}",
        text(record, "title"),
        state(record),
        text(record, "workspace"),
        text(record, "directory"),
        record
            .get("received_version")
            .and_then(Value::as_usize)
            .unwrap_or(0),
        text(record, "instructions")
    )
    .map_err(|e| e.to_string())?;
    if !text(record, "context").is_empty() {
        writeln!(output, "Assignment context:\n{}", text(record, "context"))
            .map_err(|e| e.to_string())?;
    }
    if let Some(pending) = record.get("pending_instruction").and_then(Value::as_str) {
        writeln!(output, "Pending instruction (not yet received):\n{pending}")
            .map_err(|e| e.to_string())?;
    }
    writeln!(output, "Activity: {}", text(record, "activity")).map_err(|e| e.to_string())?;
    if record.get("updates_truncated") == Some(&Value::Bool(true)) {
        writeln!(
            output,
            "Showing recent visible updates; earlier text is in the child history."
        )
        .map_err(|e| e.to_string())?;
    }
    print_list(record, "updates", "Visible updates", output)?;
    for tool in record.get("tools").and_then(Value::as_array).unwrap_or(&[]) {
        writeln!(
            output,
            "  {} · {}\n  {}",
            text(tool, "name"),
            text(tool, "state"),
            text(tool, "text")
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(report) = record.get("report").filter(|v| **v != Value::Null) {
        writeln!(
            output,
            "Report v{}:\n{}",
            report.get("version").and_then(Value::as_usize).unwrap_or(0),
            text(report, "text")
        )
        .map_err(|e| e.to_string())?;
    } else {
        writeln!(output, "No result returned yet.").map_err(|e| e.to_string())?;
    }
    print_list(record, "files", "Files", output)?;
    print_list(record, "checks", "Checks", output)?;
    if !text(record, "error").is_empty() {
        writeln!(output, "Error: {}", text(record, "error")).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn print_list(
    record: &Value,
    key: &str,
    label: &str,
    output: &mut impl Write,
) -> Result<(), String> {
    let values = record.get(key).and_then(Value::as_array).unwrap_or(&[]);
    if !values.is_empty() {
        writeln!(output, "{label}:").map_err(|e| e.to_string())?;
        for value in values.iter().filter_map(Value::as_str) {
            writeln!(output, "  {value}").map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn number(record: &Value) -> usize {
    record.get("number").and_then(Value::as_usize).unwrap_or(0)
}
fn text<'a>(record: &'a Value, key: &str) -> &'a str {
    record.get(key).and_then(Value::as_str).unwrap_or("")
}
fn state(record: &Value) -> String {
    if record.get("reviewed") == Some(&Value::Bool(true)) {
        "Reviewed by the main agent".into()
    } else if text(record, "state") == "returned" {
        "Returned · pending review by main agent".into()
    } else if text(record, "state") == "failed" {
        "Failed · pending review by main agent".into()
    } else {
        text(record, "state").into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_inspection_keeps_returned_work_unreviewed_and_shows_failure_evidence() {
        let record = Value::object([
            ("number", Value::number(2)),
            ("title", Value::string("Check search")),
            ("state", Value::string("failed")),
            ("received_version", Value::number(1)),
            ("instructions", Value::string("Check empty input")),
            (
                "pending_instruction",
                Value::string("Also check whitespace"),
            ),
            (
                "updates",
                Value::Array(vec![Value::string("Found the empty-query path")]),
            ),
            ("files", Value::Array(vec![Value::string("src/search.rs")])),
            (
                "checks",
                Value::Array(vec![Value::string("cargo test failed")]),
            ),
            ("error", Value::string("Backend connection interrupted")),
            ("reviewed", Value::Bool(false)),
        ]);
        let records = vec![record];
        let before = records.clone();
        let mut output = Vec::new();
        render(&records, "a2", &mut output).unwrap();
        let text = String::from_utf8(output).unwrap();
        for expected in [
            "AGENT 2",
            "Failed · pending review by main agent",
            "Check empty input",
            "not yet received",
            "Found the empty-query path",
            "src/search.rs",
            "cargo test failed",
            "Backend connection interrupted",
            "No result returned yet.",
        ] {
            assert!(text.contains(expected), "missing {expected}: {text}");
        }
        assert_eq!(records, before);
        assert!(render(&records, "99", &mut Vec::new()).is_err());
        assert!(render(&records, "stop 2", &mut Vec::new()).is_err());
    }

    #[test]
    fn plain_empty_inspection_explains_current_scope() {
        let mut output = Vec::new();
        render(&[], "", &mut output).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), "No open assignments.\n");
    }
}
