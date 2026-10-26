use super::{Body, content, field, line_rows, preview};
use crate::{
    json::Value,
    tui::{
        line::Line,
        text,
        theme::{BODY, CODE_TEXT, MUTED, TOOL_DIM},
    },
};

pub(super) struct Entry {
    pub text: String,
    pub stream: &'static str,
}

/// Non-blank output lines. A failure leads with stderr.
pub(super) fn entries(result: &Value, failed: bool) -> Vec<Entry> {
    let lines = |stream: &'static str| {
        text::clean(field(result, stream))
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| Entry {
                text: line.trim_end().to_owned(),
                stream,
            })
            .collect::<Vec<_>>()
    };
    let (first, second) = if failed {
        ("stderr", "stdout")
    } else {
        ("stdout", "stderr")
    };
    let mut entries = lines(first);
    entries.extend(lines(second));
    entries
}

/// Every output line as captured, blank lines included.
fn exact(result: &Value, failed: bool) -> Vec<Entry> {
    let order = if failed {
        ["stderr", "stdout"]
    } else {
        ["stdout", "stderr"]
    };
    order
        .into_iter()
        .flat_map(|stream| {
            content::payload(field(result, stream))
                .into_iter()
                .map(move |text| Entry { text, stream })
        })
        .collect()
}

/// The line that names the result: the first error of a failure, otherwise
/// the last line of stdout.
pub(super) fn headline(entries: &[Entry], failed: bool) -> Option<usize> {
    if failed
        && entries
            .first()
            .is_some_and(|entry| entry.stream == "stderr")
    {
        return Some(0);
    }
    entries
        .iter()
        .rposition(|entry| entry.stream == "stdout")
        .or_else(|| entries.len().checked_sub(1))
}

pub(super) fn render(
    result: &Value,
    failed: bool,
    skip: Option<usize>,
    columns: usize,
    expanded: bool,
) -> Body {
    // The compact lead names the result; Ctrl+O shows the output exactly.
    let entries: Vec<_> = if expanded {
        exact(result, failed).into_iter().enumerate().collect()
    } else {
        entries(result, failed)
            .into_iter()
            .enumerate()
            .filter(|(index, _)| Some(*index) != skip)
            .collect()
    };
    let range = if expanded { 0..entries.len() } else { 0..0 };
    let both = entries
        .first()
        .zip(entries.last())
        .is_some_and(|(first, last)| first.1.stream != last.1.stream);
    let mut previous = "";
    let mut rows = Vec::new();
    for (index, entry) in &entries[range.clone()] {
        if expanded && both && previous != entry.stream {
            rows.extend(line_rows(
                &Line::new(entry.stream, TOOL_DIM),
                columns,
                index * 2 + 2,
                "    ",
                "    ",
                MUTED,
            ));
        }
        previous = entry.stream;
        let style = if entry.stream == "stderr" && !failed {
            MUTED
        } else if entry.stream == "stdout" {
            CODE_TEXT
        } else {
            BODY
        };
        rows.extend(line_rows(
            &preview(
                Line::new(&entry.text, style),
                columns.saturating_sub(4),
                expanded,
            ),
            columns,
            index * 2 + 3,
            "    ",
            "    ",
            MUTED,
        ));
    }
    Body {
        rows,
        hidden: entries.len() - range.len(),
        hidden_hunks: 0,
        unit: "lines",
    }
}
