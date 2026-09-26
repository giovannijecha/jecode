//! A slash command in the transcript: the line as typed, then its receipt as
//! the single branch of a tree, with any facts aligned under the result.
//!
//! ```text
//!  /model son
//!  └─ ✓ Model set to sonnet-5 · was opus-5.5
//! ```
use super::block::MARGIN;
use super::glyph::Glyphs;
use super::model::{Receipt, Status};
use super::style::{Row, Tone};
use super::text;
use super::tool_view::mark;

pub fn rows(receipt: &Receipt, width: usize, glyph: &Glyphs) -> Vec<Row> {
    let room = width.saturating_sub(2 * MARGIN.len());
    let input = text::clip(receipt.input.trim(), room, glyph.ellipsis);
    let (name, args) = input.split_once(' ').unwrap_or((&input, ""));
    let mut head = Row::new(MARGIN, Tone::Text);
    head.push(name, Tone::Accent);
    if !args.is_empty() {
        head.push(" ", Tone::Text).push(args, Tone::Text);
    }

    let (symbol, tone) = mark(receipt.status, glyph, "");
    let mut result = Row::new(MARGIN, Tone::Text);
    result
        .push(glyph.last, Tone::Muted)
        .push(" ", Tone::Text)
        .push(symbol, tone)
        .push(" ", Tone::Text);
    let used = text::width(&result.text) + MARGIN.len();
    let body = text::clip(&receipt.result, width.saturating_sub(used), glyph.ellipsis);
    let body_tone = match receipt.status {
        Status::Failed => Tone::Error,
        _ => Tone::Text,
    };
    result.push(&body, body_tone);
    if !receipt.note.is_empty() {
        let note = format!(" {} {}", glyph.dot, receipt.note);
        let left = width.saturating_sub(text::width(&result.text) + MARGIN.len());
        result.push(&text::clip(&note, left, glyph.ellipsis), Tone::Muted);
    }

    let mut out = vec![head, result];
    // Facts start under the result text: margin, the tree's width, 3 cells.
    let lead = " ".repeat(MARGIN.len() + text::width(glyph.last) + 3);
    let column = receipt
        .facts
        .iter()
        .map(|(key, _)| text::width(key))
        .max()
        .unwrap_or(0);
    for (key, value) in &receipt.facts {
        let mut row = Row::new(lead.as_str(), Tone::Text);
        let gap = column - text::width(key) + 3;
        let left = width.saturating_sub(lead.len() + column + 3 + MARGIN.len());
        row.push(key, Tone::Accent)
            .push(&" ".repeat(gap), Tone::Text)
            .push(&text::clip(value, left, glyph.ellipsis), Tone::Muted);
        out.push(row);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::lab::glyph::UNICODE;

    fn receipt(status: Status, facts: &[(&str, &str)]) -> Receipt {
        Receipt {
            input: "/model son".into(),
            status,
            result: "Model set to sonnet-5".into(),
            note: "was opus-5.5".into(),
            facts: facts
                .iter()
                .map(|(key, value)| ((*key).into(), (*value).into()))
                .collect(),
        }
    }

    #[test]
    fn line_then_single_branch_receipt() {
        let rows = rows(&receipt(Status::Done, &[]), 60, &UNICODE);
        let texts: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
        assert_eq!(
            texts,
            [" /model son", " └─ ✓ Model set to sonnet-5 · was opus-5.5"]
        );
        assert_eq!(rows[0].spans[0], (1..7, Tone::Accent));
    }

    #[test]
    fn facts_align_under_the_result() {
        let facts = [
            ("/model [name]", "Switch model"),
            ("/help", "List commands"),
        ];
        let rows = rows(&receipt(Status::Done, &facts), 60, &UNICODE);
        assert_eq!(rows[2].text, "      /model [name]   Switch model");
        assert_eq!(rows[3].text, "      /help           List commands");
    }

    #[test]
    fn failures_and_warnings_change_mark_and_tone() {
        let failed = rows(&receipt(Status::Failed, &[]), 60, &UNICODE);
        assert!(failed[1].text.starts_with(" └─ ✗ "));
        assert!(failed[1].spans.contains(&(12..33, Tone::Error)));
        let warned = rows(&receipt(Status::Warned, &[]), 60, &UNICODE);
        assert!(warned[1].text.starts_with(" └─ ! "));
    }
}
