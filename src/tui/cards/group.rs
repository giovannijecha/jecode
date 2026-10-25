use super::{Card, Status, Tool, field, hint, line_rows, skill_name, summary, title};
use crate::{
    json::Value,
    tui::{
        line::Line,
        text,
        theme::{BODY, MUTED, TOOL_DIM},
    },
};

/// A successful line read can share a card with the reads around it.
pub fn groupable(tool: &Tool<'_>) -> bool {
    tool.name == "read"
        && Status::of(tool) == Status::Done
        && skill_name(field(tool.arguments, "path")).is_none()
        && tool.arguments.get("byte_offset").is_none()
        && tool.result.is_some_and(|result| {
            result.get("content").and_then(Value::as_str).is_some()
                && result.get("view").is_none()
                && result.get("offset") != Some(&Value::Null)
        })
}

/// `● Read(N files)`: the file names and total lines, or one row per read
/// when expanded. Pages of one file keep its path as the target.
pub fn render(reads: &[Tool<'_>], columns: usize, keycap: bool) -> Card {
    let expanded = reads.iter().any(|tool| tool.presentation.expanded);
    let mut paths: Vec<String> = Vec::new();
    for tool in reads {
        let path = text::clean(field(tool.arguments, "path"));
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    let target = match paths.as_slice() {
        [path] => path.clone(),
        paths => format!("{} files", paths.len()),
    };
    let mut rows = title::rows("Read", &target, Status::Done.style(), columns, expanded);
    let lines = |tool: &Tool<'_>| tool.result.map_or(0, summary::lines);
    if expanded {
        for (index, tool) in reads.iter().enumerate() {
            let mut row = Line::new(&text::clean(field(tool.arguments, "path")), BODY);
            let read = tool
                .result
                .map(|result| summary::read(tool.arguments, result))
                .unwrap_or_default();
            row.push(&format!(" · {}", read.trim_start_matches("Read ")), MUTED);
            rows.push(
                row.at_source(index + 1)
                    .gutter(if index == 0 { "  ⎿ " } else { "    " }, TOOL_DIM)
                    .shortened(columns),
            );
        }
        return Card {
            rows,
            hidden: false,
        };
    }
    // Show the shortest path suffix that distinguishes each file, including
    // its drive when Windows paths have identical parent directories.
    let names: Vec<_> = paths
        .iter()
        .map(|path| {
            let parts: Vec<_> = path.rsplit(['/', '\\']).collect();
            for depth in 1..=parts.len() {
                if paths.iter().all(|other| {
                    other == path
                        || !other
                            .rsplit(['/', '\\'])
                            .take(depth)
                            .eq(parts[..depth].iter().copied())
                }) {
                    let separator = if path.contains('\\') { "\\" } else { "/" };
                    return parts[..depth]
                        .iter()
                        .rev()
                        .copied()
                        .collect::<Vec<_>>()
                        .join(separator);
                }
            }
            path.clone()
        })
        .collect();
    let total: usize = reads.iter().map(lines).sum();
    let mut names = if paths.len() == 1 {
        Line::new(&format!("{} reads", reads.len()), MUTED)
    } else {
        Line::new(&names.join(", "), BODY)
    };
    names.push(&format!(" · {total} lines"), MUTED);
    if paths.len() > 1 && reads.len() > paths.len() {
        names.push(&format!(" · {} reads", reads.len()), MUTED);
    }
    let mut summary = line_rows(&names, columns, 1, "  ⎿ ", "    ", TOOL_DIM);
    if summary.len() > 2 {
        summary.truncate(2);
        let last = summary.pop().unwrap_or_default();
        summary.push(if text::cells(&last.plain()) < columns {
            let mut last = last;
            last.push("…", MUTED);
            last
        } else {
            last.shortened(columns.saturating_sub(1))
        });
    }
    rows.extend(summary);
    if keycap {
        let last = rows.pop().unwrap_or_default();
        rows.extend(hint(last, "list", columns));
    }
    Card { rows, hidden: true }
}
