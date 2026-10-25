use super::{Body, plural, preview, wrapped};
use crate::{
    file_change::{Change, Entry, Kind},
    tui::{
        line::Line,
        syntax::Highlighter,
        text,
        theme::{BAD, GOOD, MUTED, Style, TOOL_DIM},
    },
};

mod fragments;

pub(super) fn summary(change: &Change) -> String {
    if change.created && change.added == 0 {
        return "Created an empty file".into();
    }
    if change.added == 0 && change.removed == 0 {
        return if let Some(ending) = &change.ending_change {
            format!("Changed line endings · {ending}")
        } else if change.final_newline.is_some() {
            "Updated final newline".into()
        } else {
            "No changes".into()
        };
    }
    format!(
        "Added {} {}, removed {} {}",
        change.added,
        plural(change.added),
        change.removed,
        plural(change.removed)
    )
}

pub(super) fn code_rows(
    mut code: Line,
    label: &str,
    style: Style,
    columns: usize,
    source: usize,
) -> Vec<Line> {
    code.background = None;
    code.at_source(source)
        .wrap(columns.saturating_sub(4 + text::cells(label)).max(1), false)
        .into_iter()
        .enumerate()
        .map(|(part, code)| {
            let gutter = if part == 0 {
                label.to_owned()
            } else {
                " ".repeat(text::cells(label))
            };
            let mut row = Line::new(&gutter, style);
            row.origin = code.origin;
            row.spans.extend(code.spans);
            row.gutter("    ", MUTED).shortened(columns)
        })
        .collect()
}

pub(super) fn render(change: &Change, path: &str, columns: usize, expanded: bool) -> Body {
    render_inner(change, path, columns, expanded, true)
}

pub(super) fn render_snippet(change: &Change, path: &str, columns: usize, expanded: bool) -> Body {
    render_inner(change, path, columns, expanded, false)
}

fn render_inner(
    change: &Change,
    path: &str,
    columns: usize,
    expanded: bool,
    coordinate_notice: bool,
) -> Body {
    let total: usize = change.hunks.iter().map(Vec::len).sum();
    let compact = !expanded && total > 14;
    let digits = change
        .hunks
        .iter()
        .flatten()
        .flat_map(|entry| [entry.old_line, entry.new_line])
        .flatten()
        .max()
        .map_or(0, |line| line.to_string().len());
    let mut rows = Vec::new();
    let mut shown = 0;
    let mut origin = 2;
    let mut preview_rows = 0;
    for (index, hunk) in change.hunks.iter().enumerate() {
        if compact && index > 0 {
            break;
        }
        if index > 0 {
            rows.extend(wrapped("…", MUTED, columns, origin, "    ", "    ", true));
        }
        let mut old_lexer = Highlighter::for_path(path);
        let mut new_lexer = Highlighter::for_path(path);
        let partners = fragments::partners(hunk);
        let mut pending_removed = hunk
            .iter()
            .filter(|entry| entry.kind == Kind::Removed)
            .count()
            .min(3);
        let mut pending_added = hunk
            .iter()
            .filter(|entry| entry.kind == Kind::Added)
            .count()
            .min(3);
        let mut at = 0;
        while at < hunk.len() {
            let mut end = at + 1;
            while end < hunk.len() && hunk[end].kind == hunk[at].kind {
                end += 1;
            }
            let take = if compact && end - at > 6 { 3 } else { end - at };
            let mut run_shown = 0;
            for (offset, entry) in hunk[at..end].iter().enumerate() {
                let source = text::clean(&entry.text);
                let old = (entry.kind != Kind::Added).then(|| old_lexer.line(&source));
                let new = (entry.kind != Kind::Removed).then(|| new_lexer.line(&source));
                let reserved = match entry.kind {
                    Kind::Context => pending_removed + pending_added,
                    Kind::Removed => pending_added,
                    Kind::Added => pending_removed,
                };
                if !compact || offset < take && preview_rows < 14 - reserved {
                    let code = if entry.kind == Kind::Removed {
                        old.unwrap()
                    } else {
                        new.unwrap()
                    };
                    let code = if entry.kind == Kind::Context {
                        Line::new(&source, TOOL_DIM)
                    } else {
                        fragments::highlight(
                            code,
                            &source,
                            partners[at + offset].map(|index| hunk[index].text.as_str()),
                            entry.kind,
                        )
                    };
                    let code = preview(code, columns.saturating_sub(8 + digits), expanded);
                    rows.extend(entry_rows(entry, code, digits, columns, origin));
                    shown += 1;
                    preview_rows += 1;
                    run_shown += 1;
                    match entry.kind {
                        Kind::Removed => pending_removed = pending_removed.saturating_sub(1),
                        Kind::Added => pending_added = pending_added.saturating_sub(1),
                        Kind::Context => {}
                    }
                }
                origin += 1;
            }
            if compact
                && run_shown < end - at
                && preview_rows < 14 - pending_removed - pending_added
            {
                let kind = match hunk[at].kind {
                    Kind::Context => "context",
                    Kind::Removed => "removed",
                    Kind::Added => "added",
                };
                rows.extend(wrapped(
                    &format!("… {} {kind} lines", end - at - run_shown),
                    MUTED,
                    columns,
                    origin - (end - at - run_shown),
                    "    ",
                    "    ",
                    true,
                ));
                preview_rows += 1;
            }
            at = end;
        }
    }
    if let Some(newline) = change.final_newline {
        rows.extend(wrapped(
            if newline {
                "Added final newline"
            } else {
                "Removed final newline"
            },
            MUTED,
            columns,
            origin,
            "    ",
            "    ",
            true,
        ));
        origin += 1;
    }
    if (change.added > 0 || change.removed > 0)
        && let Some(ending) = &change.ending_change
    {
        rows.extend(wrapped(
            &format!("Line endings changed · {ending}"),
            MUTED,
            columns,
            origin,
            "    ",
            "    ",
            true,
        ));
        origin += 1;
    }
    if coordinate_notice && digits == 0 && !change.hunks.is_empty() {
        rows.extend(wrapped(
            "Original file coordinates unavailable.",
            MUTED,
            columns,
            origin,
            "    ",
            "    ",
            true,
        ));
    }
    Body {
        rows,
        hidden: total - shown,
        unit: "diff rows",
    }
}

fn entry_rows(
    entry: &Entry,
    code: Line,
    digits: usize,
    columns: usize,
    source: usize,
) -> Vec<Line> {
    let (mark, style, number) = match entry.kind {
        Kind::Removed => ('−', BAD, entry.old_line),
        Kind::Added => ('+', GOOD, entry.new_line),
        Kind::Context => (' ', TOOL_DIM, entry.new_line),
    };
    let coordinate =
        number.map_or_else(|| " ".repeat(digits), |number| format!("{number:>digits$}"));
    let mut rows = code_rows(
        code,
        &format!("{coordinate} {mark}  "),
        style,
        columns,
        source,
    );
    if entry.no_newline {
        rows.extend(wrapped(
            "No newline at end of file",
            MUTED,
            columns,
            source,
            "    ",
            "    ",
            true,
        ));
    }
    rows
}

#[cfg(test)]
mod tests;
