//! Related routine calls become one bounded WORK block while folded.

use super::{Card, Tool, field};
use crate::tui::text;

mod compact;
mod facts;

pub fn groupable(tool: &Tool<'_>) -> bool {
    // Inspection and observation join chapters; actions that change work
    // (such as integration) remain standalone.
    match tool.name {
        "collaboration" => {
            return tool.arguments.get("lone") != Some(&crate::json::Value::Bool(true));
        }
        "agent" => {
            return matches!(
                field(tool.arguments, "action"),
                "list" | "inspect" | "wait" | "diff"
            );
        }
        _ => {}
    }
    matches!(
        tool.name,
        "read"
            | "list"
            | "search"
            | "bash"
            | "exec"
            | "process"
            | "web"
            | "browser"
            | "mcp"
            | "lsp"
    ) && !(tool.name == "read" && field(tool.arguments, "path").starts_with("skill:"))
}

pub fn label(tool: &Tool<'_>) -> String {
    let work = field(tool.arguments, "work")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if work.is_empty() {
        "Activity".into()
    } else {
        text::clean(&work)
    }
}

pub fn header(tools: &[Tool<'_>], columns: usize, keycap: bool) -> Card {
    compact::render(tools, columns, keycap)
}
