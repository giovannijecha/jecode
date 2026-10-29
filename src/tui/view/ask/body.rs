//! The question in front: its text in bold `strong`, its context in `muted`,
//! a blank row, then its answers, or the input row for a question without
//! options.

use super::super::panel;
use crate::{
    ask::Question,
    tui::{
        ask::Entry,
        line::Line,
        text,
        theme::{ACCENT, BADGE_IDLE, CURSOR, LIT, MUTED, SELECTED_SURFACE, STRONG, Style},
    },
};
use std::ops::Range;

const OTHER: &str = "Other";
const OTHER_FACT: &str = "Type your own answer";
const PLACEHOLDER: &str = "Type your answer";
const CHIP: &str = " recommended ";

pub(super) struct Body {
    pub rows: Vec<Line>,
    /// The cursor's rows.
    pub focus: Range<usize>,
    /// The caret's row and column.
    pub caret: Option<(usize, usize)>,
    /// The columns the typed words wrap at.
    pub text: usize,
}

struct Answers {
    rows: Vec<Line>,
    focus: Range<usize>,
    caret: Option<(usize, usize)>,
    text: usize,
}

pub(super) fn body(question: &Question, entry: &Entry, columns: usize) -> Body {
    let mut rows = Vec::new();
    paragraph(&mut rows, &question.question, STRONG.bold(), columns);
    paragraph(&mut rows, &question.context, MUTED, columns);
    rows.push(Line::default());
    let first = rows.len();
    let answers = if question.options.is_empty() {
        words(entry, columns)
    } else {
        options(question, entry, columns)
    };
    rows.extend(answers.rows);
    Body {
        rows,
        focus: first + answers.focus.start..first + answers.focus.end,
        caret: answers.caret.map(|(row, column)| (first + row, column)),
        text: answers.text,
    }
}

/// Text at column 2, wrapped two cells before the edge.
fn paragraph(rows: &mut Vec<Line>, value: &str, style: Style, columns: usize) {
    if value.is_empty() {
        return;
    }
    let width = columns.saturating_sub(4).max(1);
    for line in text::clean(value).split('\n') {
        for part in Line::new(line, style).wrap(width, true) {
            rows.push(part.indent("  ", MUTED));
        }
    }
}

/// The composer's input row: `›`, then the words wrapping like a draft.
fn words(entry: &Entry, columns: usize) -> Answers {
    let width = columns.saturating_sub(3).max(1);
    if entry.editor.text.is_empty() {
        let mut line = Line::new("› ", MUTED);
        line.push(PLACEHOLDER, MUTED);
        return Answers {
            rows: vec![line.shortened(columns).caret(2, CURSOR)],
            focus: 0..1,
            caret: Some((0, 2)),
            text: width,
        };
    }
    let layout = entry.editor.layout(width);
    let (row, column) = layout.cursor;
    let rows = layout
        .rows
        .iter()
        .enumerate()
        .map(|(index, words)| {
            let mut line = if index == 0 {
                Line::new("› ", ACCENT.bold())
            } else {
                Line::new("  ", MUTED)
            };
            line.push(&words.text, STRONG);
            if index == row {
                line = line.caret(column + 2, CURSOR);
            }
            line
        })
        .collect();
    Answers {
        rows,
        focus: row..row + 1,
        caret: Some((row, column + 2)),
        text: width,
    }
}

/// One row per option, then `Other`: the `›` cursor, the mark, the label,
/// the description in the fact column and the chip ending two cells from
/// the edge. Typed words wrap under the fact column.
fn options(question: &Question, entry: &Entry, columns: usize) -> Answers {
    let end = columns.saturating_sub(2);
    let labels: Vec<String> = question
        .options
        .iter()
        .map(|option| single_line(&option.label))
        .chain([OTHER.to_owned()])
        .collect();
    let widest = labels
        .iter()
        .map(|label| text::cells(label))
        .max()
        .unwrap_or(0);
    let fact_at = (widest + 6).min((end / 2).max(12));
    let chips = end >= text::cells(CHIP) + 14;
    let mut answers = Answers {
        rows: Vec::new(),
        focus: 0..0,
        caret: None,
        text: 1,
    };
    for (index, label) in labels.iter().enumerate() {
        let cursor = index == entry.row;
        let own = index == question.options.len();
        let picked = if own {
            entry.other
        } else {
            entry.picked.contains(&index)
        };
        let mark = match (question.is_multiple(), picked) {
            (true, true) => "☑ ",
            (true, false) => "☐ ",
            (false, true) => "◉ ",
            (false, false) => "○ ",
        };
        let mut head = Line::new(if cursor { "› " } else { "  " }, ACCENT.bold());
        head.push(mark, if picked { ACCENT.bold() } else { MUTED });
        head.push(label, STRONG);
        let pad = fact_at.saturating_sub(panel::cells(&head)).max(2);
        head.push(&" ".repeat(pad), MUTED);
        let indent = panel::cells(&head);
        let start = answers.rows.len();
        if own && !entry.editor.text.is_empty() {
            let width = end.saturating_sub(indent).max(1);
            answers.text = width;
            let layout = entry.editor.layout(width);
            for (number, words) in layout.rows.iter().enumerate() {
                let mut line = if number == 0 {
                    head.clone()
                } else {
                    Line::new(&" ".repeat(indent), MUTED)
                };
                line.push(&words.text, STRONG);
                let mut line = finish(line, columns, cursor);
                if cursor && number == layout.cursor.0 {
                    let column = indent + layout.cursor.1;
                    line = line.caret(column, CURSOR);
                    answers.caret = Some((answers.rows.len(), column));
                }
                answers.rows.push(line);
            }
        } else {
            let chip = chips && !own && question.options[index].recommended;
            let chip_cells = if chip { text::cells(CHIP) + 2 } else { 0 };
            let fact = if own {
                OTHER_FACT.to_owned()
            } else {
                single_line(&question.options[index].description)
            };
            let mut line = head;
            line.push(&fact, MUTED);
            let mut line = line.shortened(end.saturating_sub(chip_cells));
            if chip {
                let used = panel::cells(&line);
                line.push(
                    &" ".repeat(end.saturating_sub(used + chip_cells - 2)),
                    MUTED,
                );
                line.push_on(CHIP, LIT, BADGE_IDLE);
            }
            let mut line = finish(line, columns, cursor);
            if cursor && own {
                line = line.caret(indent, CURSOR);
                answers.caret = Some((answers.rows.len(), indent));
            }
            answers.rows.push(line);
        }
        if cursor {
            answers.focus = start..answers.rows.len();
        }
    }
    answers
}

/// The row padded to the edge; the cursor's row on the selected surface.
fn finish(line: Line, columns: usize, cursor: bool) -> Line {
    let fill = cursor.then_some(SELECTED_SURFACE);
    let line = panel::fit(line, columns, fill);
    match fill {
        Some(fill) => panel::surfaced(line, fill),
        None => line,
    }
}

fn single_line(value: &str) -> String {
    text::clean(value).replace('\n', " ")
}
