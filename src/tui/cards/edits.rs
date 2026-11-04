//! One UPDATE card for an edit of several files (B19). The target is the
//! paths in call order and the fact sums what changed. Below it each file is
//! a section from column 2, its path in `muted` with its counts above its
//! usual diff; collapsed, the first three diffs open and the rest are
//! counted. Each file applied all or nothing, so a failed file is one row in
//! call order and the badge turns `bad`.

use super::{
    Card, Status, Tool, badge, content, diff, edit_fallback, field, hint, line_rows, preview,
    summary, text, title,
};
use crate::{
    file_change::Change,
    json::Value,
    tui::{
        line::Line,
        theme::{BAD, DIM, GOOD, MUTED},
    },
};

/// Diffs a collapsed card opens; later files keep their heading row.
const OPEN_DIFFS: usize = 3;

pub(super) fn card(tool: Tool<'_>, columns: usize, keycap: bool) -> Card {
    let expanded = tool.presentation.expanded;
    let paths = crate::events::edited(tool.arguments);
    let entries = tool
        .result
        .and_then(|result| result.get("files"))
        .and_then(Value::as_array)
        .unwrap_or(&[]);
    let failed = entries
        .iter()
        .filter(|entry| entry.get("error").is_some())
        .count();
    let status = match Status::of(&tool) {
        Status::Done if failed > 0 => Status::Failed,
        status => status,
    };
    let mut rows = title::rows("Update", &paths.join(", "), status, columns, expanded);
    let titled = rows.len();
    let fact = preview(
        fact(&tool, paths.len(), entries, failed),
        columns.saturating_sub(4),
        expanded,
    );
    // The fact joins a one-row title when it fits; otherwise it hangs.
    match rows.as_mut_slice() {
        [title] if text::cells(&title.plain()) + 2 + text::cells(&fact.plain()) <= columns => {
            title.push("  ", MUTED);
            title.spans.extend(fact.spans);
        }
        _ => rows.extend(line_rows(&fact, columns, 1, "  ⎿ ", "    ", DIM)),
    }
    badge::hang(&mut rows[titled..], true);
    let mut hidden = None;
    if tool
        .result
        .is_some_and(|result| result.get("error").is_none())
    {
        let sections = sections(&tool, entries, paths.len() == 1, columns, expanded);
        hidden = sections.hidden.map(|index| rows.len() + index);
        rows.extend(sections.rows);
    }
    if keycap && expanded {
        // `collapse` follows the last title row when the first is full, so
        // a wrapped title is never split.
        let mut first = hint(rows.remove(0), "collapse", columns);
        let own = (first.len() > 1).then(|| first.pop()).flatten();
        rows.splice(0..0, first);
        if let Some(own) = own {
            rows.insert(titled, own);
        }
    } else if keycap && let Some(index) = hidden {
        let row = rows.remove(index);
        rows.splice(index..index, hint(row, "expand", columns));
    }
    Card {
        rows,
        hidden: hidden.is_some(),
    }
}

/// `3 files · +9 −1 lines`, ` · N failed` in `bad`; one file keeps today's
/// fact, and a call that changed nothing says so on the failed card.
fn fact(tool: &Tool<'_>, files: usize, entries: &[Value], failed: usize) -> Line {
    let Some(result) = tool.result else {
        let elapsed = summary::line(tool, Status::Running, None);
        if files < 2 {
            return elapsed;
        }
        let mut line = Line::new(&format!("{files} files"), DIM);
        if !elapsed.spans.is_empty() {
            line.push(" · ", DIM);
            line = line.append(elapsed);
        }
        return line;
    };
    if result.get("error").is_some() {
        return summary::line(tool, Status::Failed, None);
    }
    let written = entries.iter().filter(|entry| written(entry)).count();
    if written == 0 {
        return if failed > 0 {
            Line::new(&format!("No files changed · {failed} failed"), BAD)
        } else {
            Line::new("No changes", MUTED)
        };
    }
    let mut line = if files == 1 {
        match tool.change.and_then(|change| change.files.first()) {
            Some((_, change)) => summary::changed(change),
            None => Line::new(&replacements(tool.arguments.get("files")), MUTED),
        }
    } else {
        let (added, removed) = tool
            .change
            .map_or((0, 0), |change| (change.added, change.removed));
        let noun = if written == 1 { "file" } else { "files" };
        Line::new(&format!("{written} {noun} · "), MUTED).append(counts(added, removed, true))
    };
    if failed > 0 {
        line.push(&format!(" · {failed} failed"), BAD);
    }
    line
}

struct Sections {
    rows: Vec<Line>,
    /// The last row that counts what Ctrl+O would add.
    hidden: Option<usize>,
}

/// Each file in call order from column 2. A lone file needs no heading:
/// the title names it.
fn sections(
    tool: &Tool<'_>,
    entries: &[Value],
    single: bool,
    columns: usize,
    expanded: bool,
) -> Sections {
    let mut rows = Vec::new();
    let mut hidden = None;
    let mut origin = 2;
    let mut opened = 0;
    let (mut more, mut more_rows) = (0, 0);
    for entry in entries {
        let path = field(entry, "path");
        if let Some(error) = entry.get("error").and_then(Value::as_str) {
            rows.extend(failure(path, error, columns, expanded, origin));
            origin += 1;
            continue;
        }
        let change = tool.change.and_then(|change| change.file(path));
        let arguments = argument(tool.arguments, path);
        if !single {
            rows.push(heading(
                path,
                tail(entry, change, arguments),
                columns,
                origin,
            ));
            origin += 1;
        }
        let notices = content::notices("edit", entry, columns, expanded);
        if !written(entry) || (!expanded && opened == OPEN_DIFFS) {
            if written(entry) {
                more += 1;
                more_rows += change.map_or(0, |change| change.hunks.iter().map(Vec::len).sum());
            }
            rows.extend(notices);
            continue;
        }
        opened += 1;
        let body = match change {
            Some(change) => Some(diff::render(change, path, columns, expanded)),
            None => arguments
                .and_then(|arguments| edit_fallback::body(arguments, entry, columns, expanded)),
        };
        if let Some(mut body) = body {
            for row in &mut body.rows {
                if let Some(source) = &mut row.origin {
                    source.line += origin - 2;
                }
            }
            origin = body
                .rows
                .iter()
                .filter_map(|row| row.origin.map(|source| source.line + 1))
                .max()
                .unwrap_or(origin);
            let label = (body.hidden > 0).then(|| body.more());
            rows.extend(body.rows);
            if let Some(label) = label {
                rows.extend(line_rows(
                    &Line::new(&label, DIM),
                    columns,
                    origin,
                    "  ",
                    "  ",
                    DIM,
                ));
                hidden = Some(rows.len() - 1);
                origin += 1;
            }
        }
        rows.extend(notices);
    }
    if more > 0 {
        let noun = if more == 1 { "diff" } else { "diffs" };
        let label = Line::new(&format!("+{more} {noun} · {more_rows} rows"), DIM);
        rows.extend(line_rows(&label, columns, origin, "  ", "  ", DIM));
        hidden = Some(rows.len() - 1);
    }
    Sections { rows, hidden }
}

/// A section's heading: the path clipped first so its counts stay.
fn heading(path: &str, tail: Line, columns: usize, origin: usize) -> Line {
    let room = columns
        .saturating_sub(4 + text::cells(&tail.plain()))
        .max(1);
    let mut line = Line::new(&text::clean(path), MUTED).shortened(room);
    if !tail.spans.is_empty() {
        line.push("  ", MUTED);
        line = line.append(tail);
    }
    line.at_source(origin).gutter("  ", DIM).shortened(columns)
}

fn tail(entry: &Value, change: Option<&Change>, arguments: Option<&Value>) -> Line {
    match change {
        _ if !written(entry) => Line::new("No changes", MUTED),
        Some(change) if change.added + change.removed > 0 => {
            counts(change.added, change.removed, false)
        }
        Some(change) => Line::new(&diff::summary(change), MUTED),
        None => Line::new(&replacements(arguments), MUTED),
    }
}

/// `path · not changed · error` in `bad`: clipped collapsed, wrapped under
/// the path expanded.
fn failure(path: &str, error: &str, columns: usize, expanded: bool, origin: usize) -> Vec<Line> {
    let line = Line::new(
        &format!(
            "{} · not changed · {}",
            text::clean(path),
            text::clean(error)
        ),
        BAD,
    )
    .at_source(origin);
    if !expanded {
        return vec![line.gutter("  ", DIM).shortened(columns)];
    }
    line.wrap(columns.saturating_sub(4).max(1), true)
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            row.gutter(if index == 0 { "  " } else { "    " }, DIM)
                .shortened(columns)
        })
        .collect()
}

/// `+2 −1`, zero sides dropped; `unit` adds ` lines`.
fn counts(added: usize, removed: usize, unit: bool) -> Line {
    let mut line = Line::default();
    if added > 0 {
        line.push(&format!("+{added}"), GOOD);
    }
    if removed > 0 {
        if added > 0 {
            line.push(" ", MUTED);
        }
        line.push(&format!("−{removed}"), BAD);
    }
    if added + removed == 0 {
        line.push("0", MUTED);
    }
    if unit {
        line.push(&format!(" {}", super::plural(added + removed)), MUTED);
    }
    line
}

/// A file written without a full-file diff names how many replacements it took.
fn replacements(arguments: Option<&Value>) -> String {
    let arguments = match arguments {
        Some(Value::Array(files)) => files.first(),
        other => other,
    };
    let count = arguments
        .and_then(|arguments| crate::tools::edit::steps(arguments).ok())
        .map_or(0, |steps| steps.len());
    let noun = if count == 1 {
        "replacement"
    } else {
        "replacements"
    };
    format!("Applied {count} {noun}")
}

fn written(entry: &Value) -> bool {
    entry.get("bytes_written").and_then(Value::as_usize) > Some(0)
}

/// The call's entry for `path`.
fn argument<'a>(arguments: &'a Value, path: &str) -> Option<&'a Value> {
    arguments
        .get("files")?
        .as_array()?
        .iter()
        .find(|file| field(file, "path") == path)
}

#[cfg(test)]
mod tests;
