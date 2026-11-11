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
        "Read" | "List" | "Search" | "Skills" | "HTTP" | "Browse" => BADGE_READ,
        "Update" | "Save" => BADGE_EDIT,
        "Bash" | "Exec" | "Proc" => BADGE_SHELL,
        "Agents" | "Todo" | "Ask user" | "Main" => BADGE_COORD,
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

/// Eight filled cells. The tool vocabulary has even widths, so both sides match.
/// Unknown tool names are clipped to the same footprint and rounded to a cell.
fn block(word: Line, surface: Surface) -> Line {
    let word = word.shortened(BLOCK);
    let cells = text::cells(&word.plain());
    let left = (BLOCK - cells) / 2;
    let right = BLOCK - cells - left;
    let style = word.spans.first().map_or(BODY, |span| span.style);
    let mut line = Line::default();
    line.push_on(&" ".repeat(left), style, surface);
    for span in word.spans {
        line.push_on(&span.text, span.style, surface);
    }
    let last = line.spans.last().map_or(style, |span| span.style);
    line.push_on(&" ".repeat(right), last, surface);
    line
}

/// Keep variable collaborator identities outside the fixed coordination badge.
pub(super) fn agent(name: &str, status: Status) -> Line {
    let label = name.to_ascii_uppercase();
    if label == "MAIN AGENT" {
        return badge("Main", status);
    }
    let mut line = badge("Agents", status);
    if let Some(number) = label.strip_prefix("AGENT ") {
        line.push(&format!(" #{number}"), STRONG);
    } else if label != "AGENTS" {
        line.push(&format!(" {}", text::clean(name)), STRONG);
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

    const NAMES: &[&str] = &[
        "HTTP", "MCP call", "LSP call", "Read", "List", "Bash", "Save", "Search", "Update", "Proc",
        "Browse", "Skills", "Sequence", "Agents", "Todo", "Ask user", "Work", "Exec", "Main",
    ];

    const STATES: [Status; 6] = [
        Status::Done,
        Status::Running,
        Status::Background,
        Status::Failed,
        Status::Stopped,
        Status::Unknown,
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
        for status in STATES {
            for name in NAMES {
                let line = badge(name, status);
                assert_eq!(text::cells(&line.plain()), BLOCK, "{name}");
                let (filled, rest) = parts(&line);
                let word = name.to_uppercase();
                let left = filled.len() - filled.trim_start().len();
                let right = filled.len() - filled.trim_end().len();
                assert_eq!(filled.trim(), word, "{name}");
                assert_eq!(left, right, "{name}: {filled:?}");
                assert!(rest.is_empty(), "{name}");
                assert_eq!(filled.len(), BLOCK, "{name}");
            }
        }
    }

    #[test]
    fn unfamiliar_names_keep_the_eight_filled_cell_footprint() {
        for name in ["Odd", "Unknown", "Future tool with a long name", "漢字"] {
            let line = badge(name, Status::Done);
            assert_eq!(text::cells(&line.plain()), BLOCK, "{name}");
            assert!(line.spans.iter().all(|span| span.background.is_some()));
        }
    }

    #[test]
    fn collaborator_numbers_do_not_resize_or_recentre_the_badge() {
        for status in STATES {
            for number in [1, 12, 123, usize::MAX] {
                for name in [format!("AGENT {number}"), format!("Agent {number}")] {
                    let line = agent(&name, status);
                    let (filled, rest) = parts(&line);
                    assert_eq!(filled, " AGENTS ");
                    assert_eq!(rest, format!(" #{number}"));
                }
            }
            let (filled, rest) = parts(&agent("MAIN AGENT", status));
            assert_eq!(filled, "  MAIN  ");
            assert!(rest.is_empty());
            assert_eq!(agent("AGENTS", status).plain(), " AGENTS ");
            assert_eq!(agent("Agents", status).plain(), " AGENTS ");
        }
    }

    #[test]
    fn quiet_surfaces_read_strong_and_family_surfaces_keep_their_label() {
        for name in ["MCP call", "LSP call", "Work", "Unknown"] {
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
            ("Main", MUTED),
            ("MCP call", MUTED),
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
