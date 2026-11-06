//! The badge that heads every tool row, and the `│` / `└` gutter its results hang on.
use super::Status;
use crate::permissions::Level;
use crate::tui::{
    line::{Line, Span},
    motion, text,
    theme::{
        ACCENT, BADGE_BAD, BADGE_COORD, BADGE_EDIT, BADGE_IDLE, BADGE_READ, BADGE_SHELL,
        BADGE_TEXT, BADGE_WARN, BASE, BODY, DIM, KEYWORD, LIT, MUTED, STRONG, Style, Surface,
    },
};

/// The cells a badge takes, so its target starts on column 9, where diff code starts.
const BLOCK: usize = 8;

/// The family surface: read, edit, shell, coordination, then the quiet one for the rest.
fn family(name: &str) -> Surface {
    match name {
        "Read" | "List" | "Search" | "Map" | "Skill" | "Web" | "Browser" => BADGE_READ,
        "Update" | "Write" => BADGE_EDIT,
        "Bash" | "Exec" | "Process" => BADGE_SHELL,
        "Agents" | "Todo" | "Ask user" => BADGE_COORD,
        name if name.starts_with("AGENT") || name.starts_with("Agent") => BADGE_COORD,
        name if name.starts_with("MAIN AGENT") => BADGE_COORD,
        _ => BADGE_IDLE,
    }
}

/// The family colour as text, for tool names inside a Work chapter.
pub(super) fn ink(name: &str) -> Style {
    match family(name) {
        BADGE_READ => ACCENT,
        BADGE_EDIT => KEYWORD,
        BADGE_SHELL => LIT,
        _ => MUTED,
    }
}

/// The conversation's level as a chip on the family surface of what it
/// allows (B12.8): reading, editing, commands.
pub(in crate::tui) fn level(level: Level) -> Line {
    let surface = match level {
        Level::Read => BADGE_READ,
        Level::Workspace => BADGE_EDIT,
        Level::Full => BADGE_SHELL,
    };
    let mut line = Line::default();
    line.push_on(&format!(" {} ", level.name()), BADGE_TEXT.bold(), surface);
    line
}

/// The state lives on the badge: the family colour when done, the quiet surface with
/// the shimmer while running or a lit name in background, red when failed and amber
/// when stopped. The quiet surface follows the theme, so its label does too.
pub(super) fn badge(name: &str, status: Status) -> Line {
    let upper = name.to_uppercase();
    let (word, surface) = match status {
        Status::Running => (
            motion::shimmer_on_badge(&upper, motion::frame()),
            BADGE_IDLE,
        ),
        Status::Background => (Line::new(&upper, LIT.bold()), BADGE_IDLE),
        Status::Failed => (Line::new(&upper, BADGE_TEXT.bold()), BADGE_BAD),
        Status::Stopped | Status::Unknown => (Line::new(&upper, BADGE_TEXT.bold()), BADGE_WARN),
        Status::Done => {
            let surface = family(name);
            let style = if surface == BADGE_IDLE {
                STRONG
            } else {
                BADGE_TEXT
            };
            (Line::new(&upper, style.bold()), surface)
        }
    };
    block(word, surface)
}

/// One block with the word centred exactly: eight cells for an even name, seven for an
/// odd one, because a word only centres on a cell grid when its gap is even. Either way
/// the badge takes eight cells. A wider name keeps its natural width.
fn block(word: Line, surface: Surface) -> Line {
    let cells = text::cells(&word.plain());
    let side = " ".repeat(BLOCK.saturating_sub(cells) / 2);
    let side = if cells > BLOCK { " " } else { side.as_str() };
    let style = word.spans.first().map_or(BODY, |span| span.style);
    let mut line = Line::default();
    line.push_on(side, style, surface);
    for span in word.spans {
        line.push_on(&span.text, span.style, surface);
    }
    let last = line.spans.last().map_or(style, |span| span.style);
    line.push_on(side, last, surface);
    if cells < BLOCK && cells % 2 == 1 {
        line.push(" ", BODY);
    }
    line
}

/// Hangs result rows under their title: every row on `│`, the last on `└`,
/// both in DIM. Only rows that open on a tree gutter are hung; a keycap on
/// its own row keeps its place. Hung rows count again, so a growing tree
/// can close on its new last row; `closed` is false while later rows of the
/// same block continue it.
pub(in crate::tui) fn hang(rows: &mut [Line], closed: bool) {
    let hung: Vec<_> = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| tree(row))
        .map(|(index, _)| index)
        .collect();
    for (position, &index) in hung.iter().enumerate() {
        let last = closed && position + 1 == hung.len();
        rows[index].spans[0] = Span {
            text: if last { "  └ " } else { "  │ " }.into(),
            style: DIM,
            background: Some(BASE),
        };
    }
}

fn tree(row: &Line) -> bool {
    row.spans.first().is_some_and(|span| {
        span.background == Some(BASE)
            && matches!(span.text.as_str(), "  ⎿ " | "    " | "  │ " | "  └ ")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: [&str; 12] = [
        "Web", "Map", "MCP", "LSP", "Read", "List", "Bash", "Save", "Write", "Search", "Update",
        "Process",
    ];

    /// The filled cells, and whatever follows them.
    fn parts(line: &Line) -> (String, String) {
        let filled = line
            .spans
            .iter()
            .filter(|span| span.background.is_some())
            .map(|span| span.text.as_str())
            .collect();
        let rest = line
            .spans
            .iter()
            .filter(|span| span.background.is_none())
            .map(|span| span.text.as_str())
            .collect();
        (filled, rest)
    }

    #[test]
    fn every_badge_takes_eight_cells_with_its_word_exactly_centred() {
        for status in [
            Status::Done,
            Status::Running,
            Status::Failed,
            Status::Stopped,
        ] {
            for name in NAMES {
                let line = badge(name, status);
                assert_eq!(text::cells(&line.plain()), BLOCK, "{name}");
                let (filled, rest) = parts(&line);
                let word = name.to_uppercase();
                let left = filled.len() - filled.trim_start().len();
                let right = filled.len() - filled.trim_end().len();
                assert_eq!(filled.trim(), word, "{name}");
                assert_eq!(left, right, "{name}: {filled:?}");
                // Odd names leave the gap outside the block, unfilled.
                let odd = word.len() % 2 == 1;
                assert_eq!(rest, if odd { " " } else { "" }, "{name}");
                assert_eq!(filled.len(), if odd { 7 } else { 8 }, "{name}");
            }
        }
    }

    #[test]
    fn a_wider_name_keeps_its_natural_width() {
        let line = badge("MAIN AGENT", Status::Done);
        assert_eq!(line.plain(), " MAIN AGENT ");
        assert_eq!(line.spans[0].background, Some(BADGE_COORD));
    }

    #[test]
    fn quiet_surfaces_read_strong_and_family_surfaces_keep_their_label() {
        for name in ["Save", "MCP", "LSP", "Work", "Unknown"] {
            let line = badge(name, Status::Done);
            assert_eq!(line.spans[0].background, Some(BADGE_IDLE), "{name}");
            assert_eq!(line.spans[0].style, STRONG.bold(), "{name}");
        }
        let read = badge("Read", Status::Done);
        assert_eq!(read.spans[0].background, Some(BADGE_READ));
        assert_eq!(read.spans[0].style, BADGE_TEXT.bold());
        assert_eq!(read.spans.len(), 1);
    }

    #[test]
    fn chapter_names_take_the_ink_of_their_family() {
        for (name, style) in [
            ("Read", ACCENT),
            ("Update", KEYWORD),
            ("Bash", LIT),
            ("Agents", MUTED),
            ("AGENT 1", MUTED),
            ("Save", MUTED),
            ("MCP", MUTED),
        ] {
            assert_eq!(ink(name), style, "{name}");
        }
    }

    #[test]
    fn a_running_badge_rests_on_dim() {
        let line = badge("Bash", Status::Running);
        assert!(
            line.spans
                .iter()
                .all(|span| span.background == Some(BADGE_IDLE))
        );
        assert!(
            line.spans.iter().any(|span| span.style == DIM.bold()),
            "{:?}",
            line.spans
        );
    }
}
