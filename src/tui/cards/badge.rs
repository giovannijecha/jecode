//! The ` NAME ` badge that opens every tool row, and the `│` / `└` gutter
//! its results hang on.
use super::Status;
use crate::tui::{
    line::{Line, Span},
    motion,
    theme::{
        ACCENT, BADGE_BAD, BADGE_EDIT, BADGE_IDLE, BADGE_OTHER, BADGE_READ, BADGE_SHELL,
        BADGE_TEXT, BADGE_WARN, BASE, DIM, KEYWORD, LIT, S_BASE, STRONG, Style, Surface,
    },
};

/// The family surface: reads, edits, shell, and one for the rest.
fn family(name: &str) -> Surface {
    match name {
        "Read" | "Skill" | "Web" | "Browser" => BADGE_READ,
        "Update" | "Write" => BADGE_EDIT,
        "Bash" | "Exec" | "Process" => BADGE_SHELL,
        _ => BADGE_OTHER,
    }
}

/// The family colour as text, for tool names inside a Work chapter.
pub(super) fn ink(name: &str) -> Style {
    match name {
        "Update" | "Write" => KEYWORD,
        "Bash" | "Exec" | "Process" => LIT,
        _ => ACCENT,
    }
}

/// ` NAME ` at natural width. The state lives on the badge: the family
/// colour when done, the quiet surface with the shimmer while running or a
/// lit name in background, red when failed and amber when stopped. `Work`
/// heads a chapter in STRONG on the quiet surface.
pub(super) fn badge(name: &str, status: Status) -> Line {
    let upper = name.to_uppercase();
    let (style, surface) = match status {
        Status::Running => return running(&upper),
        Status::Background => (LIT, BADGE_IDLE),
        Status::Failed => (BADGE_TEXT, BADGE_BAD),
        Status::Stopped => (BADGE_TEXT, BADGE_WARN),
        Status::Done if name == "Work" => (STRONG, BADGE_IDLE),
        Status::Done => (BADGE_TEXT, family(name)),
    };
    let mut line = Line::default();
    line.push_on(&format!(" {upper} "), style.bold(), surface);
    line
}

/// The status-row shimmer, same band and speed, across the name only; the
/// padding stays on the quiet surface.
fn running(name: &str) -> Line {
    let mut line = Line::default();
    line.push_on(" ", S_BASE, BADGE_IDLE);
    for span in motion::shimmer(name, motion::frame(), true).spans {
        line.push_on(&span.text, span.style, BADGE_IDLE);
    }
    line.push_on(" ", S_BASE, BADGE_IDLE);
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
