//! Conversation work receipts: question decisions and the call creating a plan.

use super::{Card, Tool};
use crate::{
    ask::Question,
    tui::{
        line::Line,
        text,
        theme::{
            ACCENT, BAD, BADGE_BAD, BADGE_IDLE, BADGE_OTHER, BADGE_WARN, BODY, DIM, LIT, MUTED,
            STRONG, Surface,
        },
    },
};

fn badge(label: &str, surface: Surface) -> Line {
    let mut line = Line::new("› ", ACCENT.bold());
    line.push_on(
        &format!(" {label} "),
        if surface == BADGE_IDLE { LIT } else { STRONG },
        surface,
    );
    line
}

fn append_wrapped(
    rows: &mut Vec<Line>,
    value: &str,
    style: crate::tui::theme::Style,
    columns: usize,
) {
    let width = columns.saturating_sub(4).max(1);
    let parts: Vec<_> = Line::new(value, style).wrap(width, true);
    for (index, part) in parts.into_iter().enumerate() {
        let gutter = if index == 0 { "  └ " } else { "    " };
        rows.push(part.indent(gutter, DIM).shortened(columns));
    }
}

pub(super) fn ask(tool: Tool<'_>, columns: usize) -> Card {
    let question = Question::parse(tool.arguments).ok();
    let title = question
        .as_ref()
        .map_or("Question", |question| question.title.as_str());
    let surface = match tool.result {
        None => BADGE_IDLE,
        Some(result) if result.get("error").is_some() => BADGE_BAD,
        Some(result)
            if matches!(
                result.get("outcome").and_then(crate::json::Value::as_str),
                Some("dismissed" | "unavailable" | "cancelled")
            ) =>
        {
            BADGE_WARN
        }
        _ => BADGE_OTHER,
    };
    let mut head = badge("ASK USER", surface);
    head.push(&format!("  {title}"), STRONG);
    let mut rows = head.wrap(columns.max(1), true);
    let summary = match tool.result {
        None => "Main agent waiting · other work continues".to_owned(),
        Some(result) => match result.get("outcome").and_then(crate::json::Value::as_str) {
            Some("answered") => {
                let choice = result
                    .get("choice")
                    .and_then(crate::json::Value::as_str)
                    .and_then(|id| {
                        question
                            .as_ref()?
                            .options
                            .iter()
                            .find(|option| option.id == id)
                            .map(|option| option.label.as_str())
                    });
                let answer = result
                    .get("text")
                    .and_then(crate::json::Value::as_str)
                    .unwrap_or("");
                [choice.unwrap_or(""), answer]
                    .into_iter()
                    .filter(|part| !part.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · ")
            }
            Some("dismissed") => "No answer supplied".into(),
            Some("cancelled") => "Question cancelled · no answer supplied".into(),
            Some("unavailable") => "Interactive input unavailable".into(),
            _ if result.get("error").is_some() => result
                .get("error")
                .and_then(crate::json::Value::as_str)
                .unwrap_or("Question failed")
                .into(),
            _ => tool.summary.into(),
        },
    };
    append_wrapped(
        &mut rows,
        &summary,
        if tool.result.is_some_and(|r| r.get("error").is_some()) {
            BAD
        } else {
            MUTED
        },
        columns,
    );
    if tool.presentation.expanded
        && let Some(question) = question
    {
        append_wrapped(&mut rows, &question.question, STRONG, columns);
        if !question.context.is_empty() {
            append_wrapped(&mut rows, &question.context, MUTED, columns);
        }
        for (index, option) in question.options.iter().enumerate() {
            append_wrapped(
                &mut rows,
                &format!(
                    "{}. {}{}",
                    index + 1,
                    option.label,
                    if option.recommended {
                        " · Recommended"
                    } else {
                        ""
                    }
                ),
                ACCENT,
                columns,
            );
            if !option.description.is_empty() {
                append_wrapped(&mut rows, &option.description, MUTED, columns);
            }
        }
    }
    Card { rows, hidden: true }
}

/// The call that creates a plan: an ordinary call row with ` TODO `, the
/// plan title and its task count. It stays as written; later reads, updates
/// and the end of the plan have no row, as the plan lives in `/todo`.
pub(super) fn todo(tool: Tool<'_>, columns: usize) -> Card {
    let status = super::Status::of(&tool);
    let mut head = super::badge::badge("Todo", status);
    let name = text::cells(&head.plain());
    head.push(" ", BODY);
    head.push(&text::clean(super::field(tool.arguments, "title")), BODY);
    let count = tool
        .arguments
        .get("tasks")
        .and_then(crate::json::Value::as_array)
        .map_or(0, <[crate::json::Value]>::len);
    let fact = Line::new(&format!("  {}", text::counted(count, "task")), MUTED);
    // The title shortens before the count, unless it would vanish.
    let room = columns.saturating_sub(text::cells(&fact.plain()));
    let head = if room > name + 2 {
        head.shortened(room).append(fact)
    } else {
        head.shortened(columns)
    };
    let mut rows = vec![head.at_source(0)];
    if status == super::Status::Failed {
        rows.extend(super::failure::rows(&tool, columns, "  ⎿ "));
        super::badge::hang(&mut rows[1..], true);
    }
    Card {
        rows,
        hidden: false,
    }
}
