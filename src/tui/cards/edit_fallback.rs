use super::{Body, diff, field, wrapped};
use crate::{file_change::Change, json::Value, tui::theme::MUTED};

fn replacements(arguments: &Value) -> Option<Vec<(&str, &str)>> {
    if let Some(items) = arguments.get("edits").and_then(Value::as_array)
        && !items.is_empty()
    {
        return items
            .iter()
            .map(|item| {
                Some((
                    item.get("old_text")?.as_str()?,
                    item.get("new_text")?.as_str()?,
                ))
            })
            .collect();
    }
    Some(vec![(
        arguments.get("old_text")?.as_str()?,
        arguments.get("new_text")?.as_str()?,
    )])
}

fn snippet_change(old: &str, new: &str) -> Option<Change> {
    let mut change = Change::between(Some(old), new)?;
    for entry in change.hunks.iter_mut().flatten() {
        entry.old_line = None;
        entry.new_line = None;
        entry.no_newline = false;
    }
    change.ending_change = None;
    change.final_newline = None;
    Some(change)
}

pub(super) fn single_change(arguments: &Value) -> Option<Change> {
    let edits = replacements(arguments)?;
    (edits.len() == 1)
        .then(|| snippet_change(edits[0].0, edits[0].1))
        .flatten()
}

pub(super) fn summary(arguments: &Value, result: &Value) -> Option<String> {
    let edits = replacements(arguments)?;
    if result.get("bytes_written").and_then(Value::as_usize) == Some(0) {
        return Some("No changes".into());
    }
    Some(format!(
        "Applied {} {} · full-file diff unavailable",
        edits.len(),
        if edits.len() == 1 {
            "replacement"
        } else {
            "replacements"
        }
    ))
}

pub(super) fn body(
    arguments: &Value,
    result: &Value,
    columns: usize,
    expanded: bool,
) -> Option<Body> {
    let edits = replacements(arguments)?;
    if result.get("bytes_written").and_then(Value::as_usize) == Some(0) {
        return None;
    }
    let mut body = Body::new(Vec::new());
    let mut origin = 2;
    let shown = if expanded {
        edits.len()
    } else {
        edits.len().min(2)
    };
    for (index, (old, new)) in edits.iter().copied().take(shown).enumerate() {
        body.rows.extend(wrapped(
            &format!("Replacement {} · supplied snippets", index + 1),
            MUTED,
            columns,
            origin,
            "    ",
            "    ",
            true,
        ));
        origin += 1;
        if let Some(change) = snippet_change(old, new) {
            let mut snippet = diff::render(&change, field(arguments, "path"), columns, expanded);
            for row in &mut snippet.rows {
                if let Some(source) = &mut row.origin {
                    source.line += origin - 2;
                }
            }
            origin = snippet
                .rows
                .iter()
                .filter_map(|row| row.origin.map(|source| source.line))
                .max()
                .unwrap_or(origin)
                + 1;
            body.hidden += snippet.hidden;
            body.rows.extend(snippet.rows);
        } else {
            body.rows.extend(wrapped(
                "Snippet diff unavailable within display limits.",
                MUTED,
                columns,
                origin,
                "    ",
                "    ",
                true,
            ));
            origin += 1;
        }
    }
    if shown < edits.len() {
        let remaining = edits.len() - shown;
        body.rows.extend(wrapped(
            &format!(
                "… {remaining} more {}",
                if remaining == 1 {
                    "replacement"
                } else {
                    "replacements"
                }
            ),
            MUTED,
            columns,
            origin,
            "    ",
            "    ",
            true,
        ));
    }
    body.unit = "diff rows";
    Some(body)
}
