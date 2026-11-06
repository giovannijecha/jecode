//! The field of a sheet: its label, its value wrapping under itself, and the only caret.

use super::super::panel::Field;
use crate::tui::{
    line::Line,
    text,
    theme::{CURSOR, DIM, MUTED, STRONG},
};

/// The cells a field's value keeps beside its label before the label stacks above it.
const FIELD_MIN: usize = 18;

/// The field's rows and caret: the label, two cells, the value wrapping under itself. A
/// label too wide to leave the value room stacks above it. Below seven columns the value
/// takes the whole row, as the draft does.
pub(super) fn rows(field: &Field, columns: usize, limit: usize) -> (Vec<Line>, (usize, usize)) {
    let label = text::clean(&field.label);
    let narrow = columns < 7;
    let stacked = !narrow && columns.saturating_sub(4 + text::cells(&label)) < FIELD_MIN;
    let indent = match (narrow, stacked) {
        (true, _) => 0,
        (_, true) => 2,
        _ => 4 + text::cells(&label),
    };
    // The value ends two cells from the edge, as text does; below seven columns it has them.
    let width = columns
        .saturating_sub(indent + if narrow { 0 } else { 2 })
        .max(1);
    let mut lead = Vec::new();
    let start = if narrow {
        Line::default()
    } else if stacked {
        lead.push(Line::new(&format!("  {label}"), MUTED).shortened(columns));
        Line::new("  ", MUTED)
    } else {
        Line::new(&format!("  {label}  "), MUTED)
    };
    if field.value.is_empty() {
        let mut line = start;
        line.push(field.placeholder, DIM);
        line.push(field.suffix, MUTED);
        let row = lead.len();
        lead.push(line.shortened(columns).caret(indent, CURSOR));
        return (lead, (row, indent));
    }
    // Split at the width, finding the caret's row and cell as it passes.
    let mut parts = vec![String::new()];
    let (mut cells, mut at, mut caret) = (0, 0, None);
    for glyph in text::glyphs(&field.value) {
        if cells + glyph.cells > width {
            parts.push(String::new());
            cells = 0;
        }
        if caret.is_none() && at >= field.caret {
            caret = Some((parts.len() - 1, cells));
        }
        parts.last_mut().unwrap().push_str(glyph.text);
        cells += glyph.cells;
        at += glyph.cells;
    }
    // A caret past the last glyph needs a cell of its own.
    if caret.is_none() && indent + cells >= columns {
        parts.push(String::new());
        cells = 0;
    }
    let (row, cell) = caret.unwrap_or((parts.len() - 1, cells));
    let limit = limit.max(1);
    let first = row
        .saturating_sub(limit / 2)
        .min(parts.len().saturating_sub(limit));
    let shown = parts.len().min(first + limit);
    let offset = lead.len();
    for index in first..shown {
        let mut line = if index == first {
            start.clone()
        } else {
            Line::new(&" ".repeat(indent), MUTED)
        };
        line.push(&parts[index], STRONG);
        if index + 1 == parts.len() {
            line.push(field.suffix, MUTED);
        }
        let mut line = line.shortened(columns);
        if index == row {
            line = line.caret(indent + cell, CURSOR);
        }
        lead.push(line);
    }
    (lead, (offset + row - first, indent + cell))
}
