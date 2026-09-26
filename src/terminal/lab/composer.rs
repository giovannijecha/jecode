//! Composer chrome, unchanged in shape from jecode: an accent rule, the
//! `›` draft with a block cursor, a closing rule and the muted footer.
use super::super::editor_visual::Visual;
use super::super::menu;
use super::block::MARGIN;
use super::glyph::Glyphs;
use super::picker;
use super::style::{Row, Tone};
use super::text;
use super::unicode::clusters;

/// Session facts shown under the composer.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Footer {
    pub cwd: String,
    pub model: String,
    pub effort: String,
}

/// Suggestions shown at once while a command name is being typed.
const SUGGESTED: usize = 6;

/// A long draft (a paste, usually) keeps its first row and the rows up to
/// the cursor; skipped rows fold into counts.
const DRAFT_HEAD: usize = 1;
const DRAFT_TAIL: usize = 3;

/// `suggestion` is the highlighted row while the draft completes a slash
/// command; the list takes the footer's place until a space is typed.
/// `busy` while a turn runs: the frame recedes to muted, input still works.
/// `cursor` is a byte offset into `draft`.
pub fn rows(
    draft: &str,
    cursor: usize,
    suggestion: usize,
    busy: bool,
    width: usize,
    glyph: &Glyphs,
    footer: &Footer,
) -> Vec<Row> {
    let frame = if busy { Tone::Muted } else { Tone::Accent };
    let rule = || Row::new(glyph.rule.repeat(width), frame);
    let mut out = vec![rule()];
    out.extend(prompt(draft, cursor, frame, width, glyph));
    out.push(rule());
    match (suggestion != usize::MAX)
        .then(|| completing(draft))
        .flatten()
    {
        Some(name) => out.extend(suggestions(name, suggestion, width, glyph)),
        None => out.push(footer_row(footer, width, glyph)),
    }
    out
}

fn suggestions(name: &str, selected: usize, width: usize, glyph: &Glyphs) -> Vec<Row> {
    let commands = menu::commands();
    let names: Vec<_> = commands
        .iter()
        .map(|entry| picker::Choice {
            label: entry.label.trim_start_matches('/').into(),
            detail: String::new(),
        })
        .collect();
    let found = picker::matches(&names, name);
    if found.is_empty() {
        return vec![Row::new(
            format!("{MARGIN}  No matching command"),
            Tone::Muted,
        )];
    }
    let choices: Vec<_> = commands
        .iter()
        .map(|entry| picker::Choice {
            label: entry.label.clone(),
            detail: entry.description.clone(),
        })
        .collect();
    let column = choices
        .iter()
        .map(|choice| text::width(&choice.label))
        .max()
        .unwrap_or(0);
    let selected = selected.min(found.len() - 1);
    let start = selected.saturating_sub(SUGGESTED - 1);
    found
        .iter()
        .enumerate()
        .skip(start)
        .take(SUGGESTED)
        .map(|(index, found)| {
            // Hits index the bare name; the label starts with `/`.
            let hits: Vec<_> = found
                .hits
                .iter()
                .map(|hit| hit.start + 1..hit.end + 1)
                .collect();
            let choice = &choices[found.index];
            picker::choice_row(choice, &hits, index == selected, column, width, glyph)
        })
        .collect()
}
fn completing(draft: &str) -> Option<&str> {
    let name = draft.strip_prefix('/')?;
    (!name.contains(char::is_whitespace)).then_some(name)
}

fn prompt(draft: &str, cursor: usize, frame: Tone, width: usize, glyph: &Glyphs) -> Vec<Row> {
    let lead = MARGIN.len() + text::width(glyph.prompt) + 1;
    let mut first = Row::new(MARGIN, Tone::Text);
    first.push(glyph.prompt, frame).push(" ", Tone::Text);
    if draft.is_empty() {
        let placeholder = format!("Ask anything{}", glyph.ellipsis);
        let head = clusters(&placeholder).next().unwrap_or(" ");
        first
            .push(head, Tone::Cursor)
            .push(&placeholder[head.len()..], Tone::Muted);
        return vec![first];
    }
    // One cell stays free at the end of each row for the cursor block.
    let visual = Visual::new(draft, width.saturating_sub(lead + 2).max(1));
    let cursor = visual.stop(cursor);
    let at = cursor.row;
    let (shown, before, after) = window(visual.rows.len(), at);
    let fold = |count: usize| {
        let label = format!("{} {count} more lines", glyph.ellipsis);
        Row::new(
            format!(
                "{}{}",
                " ".repeat(lead),
                text::clip(&label, width.saturating_sub(lead), glyph.ellipsis)
            ),
            Tone::Muted,
        )
    };
    let mut out = Vec::new();
    for index in shown {
        if before > 0 && index == DRAFT_HEAD + before {
            out.push(fold(before));
        }
        let content = &visual.rows[index];
        let mut row = if index == 0 {
            first.clone()
        } else {
            Row::new(" ".repeat(lead), Tone::Text)
        };
        if index == at {
            let byte = cursor.byte.min(content.len());
            let head = clusters(&content[byte..]).next().unwrap_or(" ");
            let tail = (byte + head.len()).min(content.len());
            row.push(&content[..byte], Tone::Text)
                .push(head, Tone::Cursor)
                .push(&content[tail..], Tone::Text);
        } else {
            row.push(content, Tone::Text);
        }
        out.push(row);
    }
    if after > 0 {
        out.push(fold(after));
    }
    out
}

/// Which rows to draw: the first, then `DRAFT_TAIL` rows ending at the
/// cursor's row (or as close as the draft allows); rows skipped before and
/// after the window are counted instead.
fn window(count: usize, at: usize) -> (Vec<usize>, usize, usize) {
    if count <= DRAFT_HEAD + DRAFT_TAIL + 1 {
        return ((0..count).collect(), 0, 0);
    }
    let start = at
        .saturating_sub(DRAFT_TAIL - 1)
        .clamp(DRAFT_HEAD, count - DRAFT_TAIL);
    let mut shown: Vec<usize> = (0..DRAFT_HEAD).collect();
    shown.extend(start..start + DRAFT_TAIL);
    (shown, start - DRAFT_HEAD, count - start - DRAFT_TAIL)
}

fn footer_row(footer: &Footer, width: usize, glyph: &Glyphs) -> Row {
    let right = text::clip(
        &format!("{} {} {}", footer.model, glyph.dot, footer.effort),
        width.saturating_sub(2 * MARGIN.len()),
        glyph.ellipsis,
    );
    let room = width.saturating_sub(2 * MARGIN.len() + text::width(&right) + 2);
    let left = text::clip(&footer.cwd, room, glyph.ellipsis);
    let gap = width.saturating_sub(2 * MARGIN.len() + text::width(&left) + text::width(&right));
    Row::new(
        format!("{MARGIN}{left}{}{right}", " ".repeat(gap)),
        Tone::Muted,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::lab::glyph::UNICODE;

    fn footer() -> Footer {
        Footer {
            cwd: "~/Codex/agent".into(),
            model: "opus-5.5".into(),
            effort: "high".into(),
        }
    }

    #[test]
    fn empty_draft_shows_cursor_on_the_placeholder() {
        let rows = rows("", usize::MAX, 0, false, 32, &UNICODE, &footer());
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1].text, " › Ask anything…");
        assert_eq!(rows[1].spans[1], (5..6, Tone::Cursor));
        assert_eq!(rows[3].text, " ~/Codex/agent  opus-5.5 · high");
    }

    #[test]
    fn long_drafts_hang_and_end_with_the_cursor() {
        let rows = rows(
            "perché non funziona questa cosa",
            usize::MAX,
            0,
            false,
            20,
            &UNICODE,
            &footer(),
        );
        let prompt: Vec<&str> = rows[1..rows.len() - 2]
            .iter()
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(prompt, [" › perché non", "   funziona questa", "   cosa "]);
        assert!(rows.iter().all(|row| text::width(&row.text) <= 20));
    }

    #[test]
    fn pasted_drafts_fold_the_middle() {
        let paste = "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7";
        let rows = rows(paste, usize::MAX, 0, false, 30, &UNICODE, &footer());
        let prompt: Vec<&str> = rows[1..rows.len() - 2]
            .iter()
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(
            prompt,
            [
                " › line 1",
                "   … 3 more lines",
                "   line 5",
                "   line 6",
                "   line 7 "
            ]
        );
        assert_eq!(rows[2].tone, Tone::Muted);
        assert_eq!(
            super::rows(
                "1\n2\n3\n4\n5",
                usize::MAX,
                0,
                false,
                30,
                &UNICODE,
                &footer()
            )
            .len(),
            5 + 3
        );
    }

    #[test]
    fn cursor_sits_on_its_cluster_mid_draft() {
        let rows = rows("abc", 1, 0, false, 20, &UNICODE, &footer());
        assert_eq!(rows[1].text, " › abc");
        assert!(rows[1].spans.contains(&(6..7, Tone::Cursor)));
        // On a hard line end the block takes an empty cell.
        let lines = super::rows("ab\ncd", 2, 0, false, 20, &UNICODE, &footer());
        assert_eq!(lines[1].text, " › ab ");
        assert_eq!(lines[2].text, "   cd");
    }

    #[test]
    fn folding_keeps_the_cursor_row_in_view() {
        let paste = "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7";
        let rows = rows(paste, 8, 0, false, 30, &UNICODE, &footer());
        let prompt: Vec<&str> = rows[1..rows.len() - 2]
            .iter()
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(
            prompt,
            [
                " › line 1",
                "   line 2",
                "   line 3",
                "   line 4",
                "   … 3 more lines"
            ]
        );
        assert!(rows[2].spans.contains(&(4..5, Tone::Cursor)));
    }

    #[test]
    fn thousand_line_draft_keeps_fold_rows_inside_twenty_columns() {
        let draft = "line\n".repeat(1_000);
        let rows = rows(
            &draft,
            draft.len(),
            usize::MAX,
            false,
            20,
            &UNICODE,
            &footer(),
        );
        assert!(
            rows.iter().all(|row| text::width(&row.text) <= 20),
            "{rows:?}"
        );
        assert!(rows.iter().any(|row| row.text.contains("more")));
    }

    #[test]
    fn busy_mutes_the_frame_but_not_the_draft() {
        let rows = rows("next", usize::MAX, 0, true, 20, &UNICODE, &footer());
        assert_eq!((rows[0].tone, rows[2].tone), (Tone::Muted, Tone::Muted));
        assert_eq!(
            (rows[1].tone, &rows[1].spans[0]),
            (Tone::Text, &(1..4, Tone::Muted))
        );
    }

    #[test]
    fn slash_drafts_swap_the_footer_for_suggestions() {
        let list = rows("/e", usize::MAX, 1, false, 50, &UNICODE, &footer());
        let texts: Vec<&str> = list[3..].iter().map(|row| row.text.as_str()).collect();
        assert!(texts[0].contains("/effort") && texts[0].contains("Set reasoning"));
        assert!(texts[1].starts_with(" › /"));
        assert_eq!(list[4].tone, Tone::User);
        assert!(list[3].spans.contains(&(4..5, Tone::Accent)));
        let none = rows("/zz", usize::MAX, 0, false, 50, &UNICODE, &footer());
        assert_eq!(none[3].text, "   No matching command");
        let args = rows("/model son", usize::MAX, 0, false, 50, &UNICODE, &footer());
        assert_eq!(args[3], footer_row(&footer(), 50, &UNICODE));
    }

    #[test]
    fn footer_clips_the_path_first() {
        let row = footer_row(&footer(), 24, &UNICODE);
        assert_eq!(row.text, " ~/Co…  opus-5.5 · high");
        assert_eq!(text::width(&row.text), 23);
    }
}
