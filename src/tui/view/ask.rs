use super::panel::{self, Part, Sheet, key};
use crate::{
    ask::Request,
    tui::{
        ask::Panel,
        line::Line,
        text,
        theme::{ACCENT, DIM, MUTED, PILL_SURFACE, STRONG},
    },
};

fn detail(text: &str, style: crate::tui::theme::Style, columns: usize) -> Vec<Line> {
    Line::new(text, style)
        .wrap(panel::text_width(columns).max(1), true)
        .into_iter()
        .map(|line| panel::indent(line, columns))
        .collect()
}

pub(super) fn sheet<'a>(request: &'a Request, answer: &'a Panel, columns: usize) -> Sheet<'a> {
    let width = panel::text_width(columns);
    let mut rows = Vec::new();
    let mut option_rows = Vec::new();
    if !request.question.title.is_empty() {
        rows.extend(detail(&request.question.title, STRONG, columns));
    }
    rows.extend(detail(&request.question.question, STRONG, columns));
    if !request.question.context.is_empty() {
        rows.extend(detail(&request.question.context, MUTED, columns));
    }
    if !request.question.options.is_empty() {
        rows.push(Line::default());
    }
    for (index, option) in request.question.options.iter().enumerate() {
        option_rows.push(rows.len());
        let chosen = answer.choice.as_deref() == Some(option.id.as_str());
        let focus = answer.choices_focused && answer.selected == index;
        let mut row = Line::new(
            if focus { "› " } else { "  " },
            if focus { ACCENT } else { DIM },
        );
        row.push(
            &format!("{}. {}", index + 1, option.label),
            if chosen { ACCENT } else { STRONG },
        );
        if option.recommended {
            row.push("  Recommended", ACCENT);
        }
        rows.push(panel::choice(row, focus, columns).0);
        if focus && !option.description.is_empty() {
            rows.extend(detail(&option.description, MUTED, columns));
        }
    }
    let total = rows.len();
    let selected = option_rows.get(answer.selected).copied().unwrap_or(0);
    Sheet {
        title: panel::title(None, "Ask user", "answer required"),
        aside: Line::new("Waiting for you", MUTED),
        field: None,
        draft_caret: false,
        head: vec![],
        body: Box::new(move |room| {
            let start = if total > room && selected > 0 {
                selected
                    .saturating_sub(room.saturating_sub(1))
                    .min(total - room)
            } else {
                0
            };
            rows.iter()
                .skip(start)
                .take(room)
                .cloned()
                .enumerate()
                .map(|(index, line)| {
                    let part = if answer.choices_focused && start + index == selected {
                        Part::Focus
                    } else {
                        Part::Row((start + index).abs_diff(selected))
                    };
                    (line, part)
                })
                .collect()
        }),
        total: Some(total),
        tail: Box::new(move |overflow| {
            panel::keys(
                &[
                    key("Tab", "answer / choices"),
                    key("↑↓", "choose").scrolling(),
                    key("Enter", "send answer"),
                    key("Ctrl+D", "dismiss"),
                    key("Esc", "chat"),
                ],
                width,
                overflow,
            )
        }),
        confirming: false,
    }
}

pub(super) fn status(columns: usize, hidden: bool) -> Line {
    let mut line = Line::default();
    line.push_on(" ? ", ACCENT.bold(), PILL_SURFACE);
    line.push_on(" Waiting for you ", STRONG, PILL_SURFACE);
    if hidden && text::cells(&line.plain()) + text::cells("  /ask answer") <= columns {
        line.push("  /ask", ACCENT);
        line.push(" answer", MUTED);
    }
    line.shortened(columns)
}
