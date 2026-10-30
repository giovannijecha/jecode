//! `/todo`: the model's plan as a B5 menu, every task in plan order with its
//! state and owner. There is nothing to select, open or type.

use super::panel::{self, Part, Sheet, key};
use crate::{
    json::Value,
    todo::{self, Phase, Plan},
    tui::{
        cards,
        line::Line,
        state::State,
        text,
        theme::{ACCENT, BAD, BODY, DIM, FAINT, GOOD, MUTED, STRONG, Style},
        todo::Panel,
    },
};

/// Under this width the numbers leave and `awaiting review` shortens.
const NARROW: usize = 64;
pub(in crate::tui) const NO_PLAN: &str = "No plan yet. The model writes one for multi-step work.";

/// A state's B3 mark, title and word styles, and its cell on the rule's
/// strip; cancelled tasks have none.
struct Look {
    mark: (&'static str, Style),
    title: Style,
    word: Style,
    strip: Option<(&'static str, Style)>,
}

fn look(phase: Phase) -> Look {
    let (mark, title, word, strip) = match phase {
        Phase::Done => (("☑ ", GOOD), MUTED, GOOD, Some(("▰", GOOD))),
        Phase::InProgress => (
            ("☐ ", ACCENT.bold()),
            STRONG.bold(),
            ACCENT,
            Some(("▰", ACCENT)),
        ),
        Phase::Waiting | Phase::AwaitingReview => {
            (("☐ ", MUTED), STRONG, MUTED, Some(("▰", MUTED)))
        }
        Phase::Failed => (("☐ ", BAD.bold()), STRONG, BAD, Some(("▰", BAD))),
        Phase::NotStarted => (("☐ ", MUTED), BODY, MUTED, Some(("▱", FAINT))),
        Phase::Cancelled => (("☐ ", FAINT), DIM.strike(), DIM, None),
    };
    Look {
        mark,
        title,
        word,
        strip,
    }
}

/// One strip cell per task that still counts, then `N of M done`.
fn aside(plan: &Plan, records: &[Value]) -> Line {
    let mut strip = Line::default();
    let (mut done, mut counted) = (0, 0);
    for task in &plan.tasks {
        let phase = Phase::of(task, todo::owner(task, records));
        if let Some((cell, style)) = look(phase).strip {
            strip.push(cell, style);
            counted += 1;
            done += usize::from(phase == Phase::Done);
        }
    }
    if counted > 0 {
        strip.push(&format!(" {done} of {counted} done"), MUTED);
    }
    strip
}

/// Plan-ordered rows: the number (dropped when narrow), the mark and title
/// from column 2, then the owner badge and the state word in aligned
/// columns, the state column ending two cells from the edge.
pub(in crate::tui) fn rows(plan: &Plan, records: &[Value], columns: usize) -> Vec<Line> {
    let narrow = columns < NARROW;
    let tasks: Vec<_> = plan
        .tasks
        .iter()
        .map(|task| {
            let owner = todo::owner(task, records);
            let lifecycle = owner
                .and_then(|record| record.get("state").and_then(Value::as_str))
                .unwrap_or("");
            let badge =
                todo::owner_label(task, owner).map(|label| cards::agent_badge(&label, lifecycle));
            (task, Phase::of(task, owner), badge)
        })
        .collect();
    let words = tasks
        .iter()
        .map(|(_, phase, _)| text::cells(phase.word(narrow)))
        .max()
        .unwrap_or(0);
    let owners = tasks
        .iter()
        .filter_map(|(_, _, badge)| badge.as_ref().map(panel::cells))
        .max()
        .unwrap_or(0);
    let right = words + if owners > 0 { owners + 2 } else { 0 };
    let end = columns.saturating_sub(2);
    let room = end.saturating_sub(right);
    let mut number = 0;
    tasks
        .into_iter()
        .map(|(task, phase, badge)| {
            let look = look(phase);
            let mut left = Line::new("  ", MUTED);
            if !narrow {
                let column = if phase == Phase::Cancelled {
                    "    ".to_owned()
                } else {
                    number += 1;
                    format!("{number:>2}  ")
                };
                left.push(&column, DIM);
            }
            left.push(look.mark.0, look.mark.1);
            left.push(&text::clean(&task.title), look.title);
            let mut tail = Line::default();
            if owners > 0 {
                let badge = badge.unwrap_or_default();
                let gap = owners - panel::cells(&badge) + 2;
                tail = badge;
                tail.push(&" ".repeat(gap), MUTED);
            }
            tail.push(phase.word(narrow), look.word);
            let row = panel::fit(left.shortened(room.saturating_sub(2)), room, None).append(tail);
            panel::fit(row, end, None)
        })
        .collect()
}

pub(super) fn sheet<'a>(state: &'a State, panel_state: &'a Panel, columns: usize) -> Sheet<'a> {
    let Some(plan) = state.todo_plan.as_ref() else {
        return message(panel::title(None, "Todo", ""), NO_PLAN, columns);
    };
    let title = panel::title(None, "Todo", &plan.title);
    if plan.tasks.is_empty() {
        return message(title, "This plan has no tasks.", columns);
    }
    let records = &state.collaboration;
    let rows = rows(plan, records, columns);
    let total = rows.len();
    let width = panel::text_width(columns);
    Sheet {
        title,
        aside: aside(plan, records),
        field: None,
        draft_caret: false,
        head: Vec::new(),
        body: Box::new(move |room| {
            let start = panel_state.start(plan, records, room);
            let shown = rows
                .iter()
                .skip(start)
                .take(room)
                .cloned()
                .map(|line| (line, None))
                .collect();
            panel::window(shown, start, total, room, columns)
                .into_iter()
                .enumerate()
                .map(|(index, line)| (line, Part::Row(index)))
                .collect()
        }),
        total: Some(total),
        tail: Box::new(move |overflow| {
            panel::keys(
                &[key("↑↓", "scroll").scrolling(), key("Esc", "close")],
                width,
                overflow,
            )
        }),
        confirming: false,
    }
}

/// One dim line in place of the plan, and only the way out.
fn message<'a>(title: Vec<Line>, value: &str, columns: usize) -> Sheet<'a> {
    let width = panel::text_width(columns);
    let rows: Vec<Line> = Line::new(value, DIM)
        .wrap(width.max(1), true)
        .into_iter()
        .map(|line| panel::indent(line, columns))
        .collect();
    let total = rows.len();
    Sheet {
        title,
        aside: Line::default(),
        field: None,
        draft_caret: false,
        head: Vec::new(),
        body: Box::new(move |_| {
            rows.iter()
                .cloned()
                .map(|line| (line, Part::Keep))
                .collect()
        }),
        total: Some(total),
        tail: Box::new(move |overflow| panel::keys(&[key("Esc", "close")], width, overflow)),
        confirming: false,
    }
}
