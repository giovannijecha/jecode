//! The root's outcome while its supervised process tree remains active.

use super::Tool;
use crate::{
    json::Value,
    tui::{
        line::Line,
        theme::{BAD, MUTED},
    },
};

pub(super) fn line(tool: &Tool<'_>, result: &Value) -> Option<Line> {
    if !matches!(tool.name, "bash" | "exec" | "process")
        || !matches!(
            crate::events::process_status(result),
            Some("running" | "stopping")
        )
    {
        return None;
    }
    let root = result.get("root_process")?;
    if root.get("status").and_then(Value::as_str) != Some("exited") {
        return None;
    }
    let code = root.get("exit_code")?;
    let style = if *code == Value::number(0) {
        MUTED
    } else {
        BAD
    };
    let label = if *code == Value::Null {
        "Root exited without code".to_owned()
    } else {
        format!("Root exited {}", code.encode())
    };
    let mut line = Line::new(&label, style);
    if let Some(count) = result
        .get("descendants")
        .and_then(|value| value.get("running"))
        .and_then(Value::as_usize)
    {
        let noun = if count == 1 {
            "descendant"
        } else {
            "descendants"
        };
        line.push(&format!(" · {count} {noun} running"), MUTED);
    } else if result
        .get("waiting_for")
        .and_then(Value::as_str)
        .is_some_and(|waiting| waiting.contains("output"))
    {
        line.push(" · output still open", MUTED);
    }
    Some(line)
}
