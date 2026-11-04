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
        theme::{BAD, DIM, MUTED},
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
        _ if tool.result.and_then(crate::events::not_run).is_some() => Status::Stopped,
        Status::Done if failed > 0 => Status::Failed,
        status => status,
    };
    let mut rows = title::rows("Update", &paths.join(", "), status, columns, expanded);
    let titled = rows.len();
    let whole = fact(&tool, paths.len(), entries, failed);
    let fact = preview(whole.clone(), columns.saturating_sub(4), expanded);
    let clipped = fact.plain() != whole.plain();
    // The fact joins a one-row title when it fits; otherwise it hangs.
    match rows.as_mut_slice() {
        [title]
            if text::cells(&title.plain()) + 2 + text::cells(&fact.plain()) <= columns
                && !fact.plain().contains('\n') =>
        {
            title.push("  ", MUTED);
            title.spans.extend(fact.spans);
        }
        _ => rows.extend(line_rows(&fact, columns, 1, "  ⎿ ", "    ", DIM)),
    }
    badge::hang(&mut rows[titled..], true);
    // Ctrl+O shows the rest of a clipped fact, so its last row names the key.
    let mut hidden = clipped.then(|| rows.len() - 1);
    if tool
        .result
        .is_some_and(|result| result.get("error").is_none())
    {
        let sections = sections(&tool, entries, paths.len() == 1, columns, expanded);
        if let Some(index) = sections.hidden {
            hidden = Some(rows.len() + index);
        }
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
/// fact, and a call that changed nothing says so on the failed card. A call
/// that never ran gives its reason in `muted`: the badge carries the state.
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
    if let Some(reason) = crate::events::not_run(result) {
        return Line::new(&text::clean(reason), MUTED);
    }
    if result.get("error").is_some() {
        return summary::line(tool, Status::Failed, None);
    }
    let written = entries
        .iter()
        .filter(|entry| written(entry, change_of(tool, entry)))
        .count();
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
        let noun = if written == 1 { "file" } else { "files" };
        let mut line = Line::new(&format!("{written} {noun}"), MUTED);
        // A file over the diff limits has no counts, and a sum of the others
        // would read as the whole call.
        let unknown = entries
            .iter()
            .any(|entry| change_of(tool, entry).is_none() && crate::events::file_changed(entry));
        if let Some(change) = tool
            .change
            .filter(|change| !unknown && change.added + change.removed > 0)
        {
            line.push(" · ", MUTED);
            line = line.append(summary::changed(change));
        }
        line
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
    // Rows stay known only while every counted file kept its diff.
    let (mut more, mut more_rows) = (0, Some(0));
    for entry in entries {
        let path = field(entry, "path");
        if let Some(error) = entry.get("error").and_then(Value::as_str) {
            rows.extend(failure(path, error, columns, expanded, origin));
            origin += 1;
            continue;
        }
        let change = change_of(tool, entry);
        let written = written(entry, change);
        let arguments = argument(tool.arguments, path);
        if !single {
            rows.push(heading(
                path,
                tail(written, change, arguments),
                columns,
                origin,
            ));
            origin += 1;
        }
        let notices = content::notices("edit", entry, columns, expanded);
        if !written || (!expanded && opened == OPEN_DIFFS) {
            if written {
                more += 1;
                more_rows = more_rows
                    .zip(change)
                    .map(|(rows, change)| rows + change.hunks.iter().map(Vec::len).sum::<usize>());
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
            origin = content::placed(&mut body.rows, origin);
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
        let mut label = format!("+{more} {noun}");
        match more_rows {
            None | Some(0) => {}
            Some(1) => label.push_str(" · 1 row"),
            Some(count) => label.push_str(&format!(" · {count} rows")),
        }
        let label = Line::new(&label, DIM);
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

fn tail(written: bool, change: Option<&Change>, arguments: Option<&Value>) -> Line {
    match change {
        _ if !written => Line::new("No changes", MUTED),
        Some(change) if change.added + change.removed > 0 => {
            summary::counts(change.added, change.removed)
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

/// A retained diff proves the file changed; without one, the result decides.
fn written(entry: &Value, change: Option<&Change>) -> bool {
    change.is_some() || crate::events::file_changed(entry)
}

fn change_of<'a>(tool: &Tool<'a>, entry: &Value) -> Option<&'a Change> {
    tool.change?.file(field(entry, "path"))
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
