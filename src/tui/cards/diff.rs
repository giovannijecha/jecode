use super::{Body, plural, wrapped};
use crate::{
    file_change::{Change, Entry, Kind},
    tui::{
        line::{Fold, Line},
        syntax::Highlighter,
        text,
        theme::{BAD, CODE_TEXT, DIFF_ADDED_SURFACE, DIFF_REMOVED_SURFACE, DIM, GOOD, MUTED},
    },
};

use std::ops::Range;

mod fragments;
mod window;

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

/// Code rows behind a `label` (line number, diff mark), wrapped at token
/// boundaries under `fold`; continuation rows keep the label's width blank.
pub(super) fn code_rows(
    mut code: Line,
    label: &Line,
    gutter: &str,
    columns: usize,
    source: usize,
    fold: Fold,
) -> Vec<Line> {
    let width = text::cells(&label.plain());
    code.background = None;
    code.at_source(source)
        .code(
            columns.saturating_sub(text::cells(gutter) + width).max(1),
            fold,
        )
        .into_iter()
        .enumerate()
        .map(|(part, code)| {
            let mut row = if part == 0 {
                label.clone()
            } else {
                Line::new(&" ".repeat(width), DIM)
            };
            row.origin = code.origin;
            row.spans.extend(code.spans);
            row.gutter(gutter, DIM).shortened(columns)
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
            rows.push(
                Line::new(&format!("{} ⋮", " ".repeat(digits + 2)), DIM)
                    .at_source(origin)
                    .gutter("  ", DIM)
                    .shortened(columns),
            );
        }
        let partners = fragments::partners(hunk);
        let lines = highlighted(hunk, &partners, path);
        let folds = window::folds(
            hunk,
            &partners,
            &lines,
            columns.saturating_sub(7 + digits).max(1),
            expanded,
            Highlighter::for_path(path).markup(),
        );
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
                let reserved = match entry.kind {
                    Kind::Context => pending_removed + pending_added,
                    Kind::Removed => pending_added,
                    Kind::Added => pending_removed,
                };
                if !compact || offset < take && preview_rows < 14 - reserved {
                    let line = at + offset;
                    rows.extend(entry_rows(
                        entry,
                        lines[line].0.clone(),
                        digits,
                        columns,
                        origin,
                        folds[line],
                    ));
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
    let hidden = total - shown;
    let hidden_hunks = if hidden > 0 {
        change.hunks.len().saturating_sub(1)
            + usize::from(shown < change.hunks.first().map_or(0, Vec::len))
    } else {
        0
    };
    Body {
        rows,
        hidden,
        hidden_hunks,
        unit: "diff rows",
    }
}

/// Each line's code, and for a paired change its fragment: context stays
/// plain, changes keep their syntax under the fragment's emphasis.
fn highlighted(
    hunk: &[Entry],
    partners: &[Option<usize>],
    path: &str,
) -> Vec<(Line, Option<Range<usize>>)> {
    let mut old_lexer = Highlighter::for_path(path);
    let mut new_lexer = Highlighter::for_path(path);
    let markup = old_lexer.markup();
    hunk.iter()
        .zip(partners)
        .map(|(entry, partner)| {
            let source = text::clean(&entry.text);
            let old = (entry.kind != Kind::Added).then(|| old_lexer.line(&source));
            let new = (entry.kind != Kind::Removed).then(|| new_lexer.line(&source));
            if entry.kind == Kind::Context {
                return (Line::new(&source, CODE_TEXT), None);
            }
            let range = partner
                .map(|other| fragments::changed(&source, &text::clean(&hunk[other].text), markup));
            let code = old.or(new).unwrap();
            (fragments::highlight(code, range.clone(), entry.kind), range)
        })
        .collect()
}

fn entry_rows(
    entry: &Entry,
    code: Line,
    digits: usize,
    columns: usize,
    source: usize,
    fold: Fold,
) -> Vec<Line> {
    let (mark, style, number) = match entry.kind {
        Kind::Removed => ('−', BAD, entry.old_line),
        Kind::Added => ('+', GOOD, entry.new_line),
        Kind::Context => (' ', DIM, entry.new_line),
    };
    // From column 2: the number in DIM at least four cells wide, then the mark.
    let width = digits + 2;
    let coordinate = number.map_or_else(|| " ".repeat(width), |number| format!("{number:>width$}"));
    let mut label = Line::new(&format!("{coordinate} "), DIM);
    label.push(&format!("{mark} "), style);
    let mut rows = code_rows(code, &label, "  ", columns, source, fold);
    if let Some(surface) = match entry.kind {
        Kind::Removed => Some(DIFF_REMOVED_SURFACE),
        Kind::Added => Some(DIFF_ADDED_SURFACE),
        Kind::Context => None,
    } {
        for row in &mut rows {
            row.background = Some(surface);
        }
    }
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
