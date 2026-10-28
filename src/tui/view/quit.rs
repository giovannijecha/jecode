//! The quit guard: the running work in the menu standard, the question in
//! place of the key row.

use super::panel::{self, Part, Sheet, key};
use crate::tui::{
    activity,
    line::Line,
    state::State,
    text,
    theme::{MUTED, STRONG},
};

/// The current turn, live, then the background processes found on Ctrl+Q.
pub(super) fn sheet<'a>(
    state: &'a State,
    processes: &'a [(String, String)],
    columns: usize,
) -> Sheet<'a> {
    let mut work: Vec<(String, String)> = state
        .activity
        .as_ref()
        .map(|turn| {
            let fact = format!(
                "{} · {}",
                super::status::verb(state),
                activity::duration(turn.started.elapsed())
            );
            ("Current turn".to_owned(), fact)
        })
        .into_iter()
        .collect();
    work.extend(processes.iter().cloned());
    let question = if state.question.is_some() && processes.is_empty() {
        "Keep the question for resume and quit?"
    } else if state.question.is_some() {
        "Stop background work, keep the question, and quit?"
    } else {
        match work.len() {
            0 | 1 => "Stop it and quit?",
            2 => "Stop both and quit?",
            _ => "Stop all and quit?",
        }
    };
    let width = panel::text_width(columns);
    Sheet {
        title: panel::title(None, "Quit", "work is running"),
        aside: Line::default(),
        field: None,
        draft_caret: false,
        head: Vec::new(),
        body: Box::new(move |room| rows(&work, columns, room)),
        total: None,
        tail: Box::new(move |_| {
            panel::confirm(
                question,
                &[key("Enter", "quit"), key("Esc", "cancel")],
                width,
            )
        }),
        confirming: true,
    }
}

/// Names at column 2 in `strong`, facts aligned after the longest name.
fn rows(work: &[(String, String)], columns: usize, room: usize) -> Vec<(Line, Part)> {
    let end = columns.saturating_sub(2);
    let names = work
        .iter()
        .map(|(name, _)| text::cells(name))
        .max()
        .unwrap_or(0)
        + 2;
    work.iter()
        .take(room)
        .enumerate()
        .map(|(index, (name, fact))| {
            let mut line = panel::marker(false, false);
            line.push(&text::clean(name), STRONG);
            if !fact.is_empty() && end >= 2 + names + 6 {
                line = panel::fit(line, 2 + names, None);
                line.push(&text::clean(fact), MUTED);
            }
            (line.shortened(end), Part::Row(index))
        })
        .collect()
}
