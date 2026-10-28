use super::panel::{self, Field, Part, Sheet, key};
use crate::{
    todo::{Plan, Task},
    tui::{
        line::Line,
        theme::{ACCENT, DIM, GOOD, MUTED, STRONG, WARNING},
        todo::Panel,
    },
};

pub(in crate::tui) fn state_label(status: &str) -> (&'static str, crate::tui::theme::Style) {
    match status {
        "pending" => ("Pending", MUTED),
        "in_progress" => ("In progress", ACCENT),
        "waiting" => ("Waiting", WARNING),
        "done" => ("Done", GOOD),
        "cancelled" => ("Cancelled", DIM),
        _ => ("Unknown", WARNING),
    }
}

pub(super) fn detail_rows(task: &Task, columns: usize) -> Vec<Line> {
    let width = panel::text_width(columns).max(1);
    let mut rows = Vec::new();
    for (text, style) in [
        (task.title.as_str(), STRONG),
        (task.id.as_str(), state_label(&task.status).1),
        (task.agent.as_deref().unwrap_or(""), DIM),
        (
            task.note.as_str(),
            if task.status == "waiting" {
                WARNING
            } else {
                MUTED
            },
        ),
    ] {
        if text.is_empty() {
            continue;
        }
        rows.extend(
            Line::new(text, style)
                .wrap(width, true)
                .into_iter()
                .map(|line| panel::indent(line, columns)),
        );
    }
    rows
}

pub(super) fn sheet<'a>(
    plan: Option<&'a Plan>,
    panel_state: &'a Panel,
    columns: usize,
) -> Sheet<'a> {
    let width = panel::text_width(columns);
    if let Some(id) = &panel_state.detail_id {
        let task = plan.and_then(|plan| plan.tasks.iter().find(|task| &task.id == id));
        let label = task.map_or("Task", |task| task.title.as_str());
        let rows = task
            .map(|task| detail_rows(task, columns))
            .unwrap_or_default();
        let total = rows.len();
        return Sheet {
            title: panel::title(Some("Todo"), label, "read only"),
            aside: Line::new(task.map_or("", |task| state_label(&task.status).0), MUTED),
            field: None,
            draft_caret: false,
            head: vec![],
            body: Box::new(move |room| {
                let start = panel_state.offset.min(total.saturating_sub(room));
                panel::window(
                    rows.iter()
                        .skip(start)
                        .take(room)
                        .cloned()
                        .map(|line| (line, None))
                        .collect(),
                    start,
                    total,
                    room,
                    columns,
                )
                .into_iter()
                .enumerate()
                .map(|(i, line)| (line, Part::Row(i)))
                .collect()
            }),
            total: Some(total),
            tail: Box::new(move |overflow| {
                panel::keys(
                    &[key("↑↓", "scroll").scrolling(), key("Esc", "back to tasks")],
                    width,
                    overflow,
                )
            }),
            confirming: false,
        };
    }
    let tasks = plan
        .map(|plan| panel_state.filtered(plan))
        .unwrap_or_default();
    let total = tasks.len();
    let (value, caret) = panel_state
        .filter
        .viewport(columns.saturating_sub(3).max(1), false);
    Sheet {
        title: panel::title(None, "Todo", "current conversation"),
        aside: Line::new(
            &format!("{} tasks", plan.map_or(0, |plan| plan.tasks.len())),
            MUTED,
        ),
        field: Some(Field {
            label: "Filter tasks".into(),
            value,
            caret,
            placeholder: "",
            suffix: "",
        }),
        draft_caret: false,
        head: vec![],
        body: Box::new(move |room| {
            if total == 0 {
                let empty = if plan.is_none() {
                    "No plan yet"
                } else {
                    "No matching tasks"
                };
                return vec![(panel::indent(Line::new(empty, DIM), columns), Part::Keep)];
            }
            let start = panel::start(panel_state.selected, total, room);
            let rows = tasks
                .iter()
                .skip(start)
                .take(room)
                .enumerate()
                .map(|(offset, task)| {
                    let index = start + offset;
                    let on = index == panel_state.selected;
                    let mut line = panel::marker(on, false);
                    line.push(&task.title, STRONG);
                    let (label, style) = state_label(&task.status);
                    let right = Line::new(label, style);
                    panel::choice(
                        panel::spread(line, right, columns.saturating_sub(2), None),
                        on,
                        columns,
                    )
                })
                .collect();
            panel::window(rows, start, total, room, columns)
                .into_iter()
                .enumerate()
                .map(|(offset, line)| {
                    (
                        line,
                        Part::Row((start + offset).abs_diff(panel_state.selected)),
                    )
                })
                .collect()
        }),
        total: Some(total),
        tail: Box::new(move |overflow| {
            panel::keys(
                &[
                    key("↑↓", "move").scrolling(),
                    key("Enter", "inspect"),
                    key("Esc", "close"),
                ],
                width,
                overflow,
            )
        }),
        confirming: false,
    }
}
