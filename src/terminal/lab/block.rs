//! Rows for one transcript block at a given width. Every block owns a one-cell
//! left margin so prose, panels and trees share a single alignment column.
use super::command_view;
use super::glyph::Glyphs;
use super::markdown;
use super::model::Block;
use super::style::{Row, Tone, indent, pad};
use super::text;
use super::tool_view;

/// Cells between the terminal edge and block content.
pub const MARGIN: &str = " ";

/// `spinner` is the current working glyph, drawn on running tool steps;
/// `expanded` shows tool output and diffs in full.
pub fn rows(
    block: &Block,
    width: usize,
    glyph: &Glyphs,
    spinner: &str,
    expanded: bool,
) -> Vec<Row> {
    match block {
        Block::User(prompt) => user(prompt, width, glyph),
        Block::Assistant(source) => assistant(source, width, glyph),
        Block::Tools(tools) => tool_view::rows(tools, width, glyph, spinner, expanded),
        Block::Command(receipt) => command_view::rows(receipt, width, glyph),
        Block::Local { text, failed } => {
            let tone = if *failed { Tone::Error } else { Tone::Muted };
            let room = width.saturating_sub(2).max(1);
            text::wrap(&text::safe(text), room)
                .into_iter()
                .map(|line| indent(Row::new(line, tone), MARGIN))
                .collect()
        }
    }
}

/// Full-width panel: a padding row, `› prompt` with hanging continuation
/// rows, and a closing padding row.
fn user(prompt: &str, width: usize, glyph: &Glyphs) -> Vec<Row> {
    let lead = MARGIN.len() + text::width(glyph.prompt) + 1;
    let blank = || pad(Row::new("", Tone::User), width);
    let mut out = vec![blank()];
    let body = text::wrap(prompt.trim_end(), width.saturating_sub(lead + 1).max(1));
    for (index, line) in body.iter().enumerate() {
        let mut row = Row::new(MARGIN, Tone::User);
        if index == 0 {
            row.push(glyph.prompt, Tone::Accent).push(" ", Tone::User);
        } else {
            row.push(&" ".repeat(lead - MARGIN.len()), Tone::User);
        }
        row.push(line, Tone::User);
        out.push(pad(row, width));
    }
    out.push(blank());
    out
}

fn assistant(source: &str, width: usize, glyph: &Glyphs) -> Vec<Row> {
    let inner = width.saturating_sub(2 * MARGIN.len()).max(1);
    let clean = text::safe(source.trim_end());
    let mut state = markdown::State::default();
    let mut out = Vec::new();
    for line in clean.split('\n') {
        state.line(line, inner, glyph, &mut out);
    }
    out.into_iter().map(|row| indent(row, MARGIN)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::lab::glyph::UNICODE;

    #[test]
    fn user_panel_is_full_width_with_a_hanging_prompt() {
        let rows = rows(
            &Block::User("rename the store now".into()),
            16,
            &UNICODE,
            "",
            false,
        );
        let texts: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "                ",
                " › rename the   ",
                "   store now    ",
                "                "
            ]
        );
        assert!(rows.iter().all(|row| row.tone == Tone::User));
    }

    #[test]
    fn assistant_rows_keep_the_margin_and_right_gutter() {
        let source = "Some prose that wraps.\n```\ncode\n```";
        let rows = rows(&Block::Assistant(source.into()), 12, &UNICODE, "", false);
        assert!(rows.iter().all(|row| row.text.starts_with(' ')));
        assert!(
            rows.iter().all(|row| text::width(&row.text) <= 11),
            "{rows:?}"
        );
        assert_eq!(rows[0].text, " Some prose");
    }
}
