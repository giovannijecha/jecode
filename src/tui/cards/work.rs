//! Conversation work receipts: question decisions and the call creating a plan.

use super::{Card, Status, Tool};
use crate::{
    ask::{Answer, Question, Reply, Request},
    tui::{
        line::Line,
        text,
        theme::{BAD, BODY, DIM, MUTED, STRONG, Style},
    },
};

/// Nothing while the question waits. Afterwards one `ASK USER` row, the
/// question's title or `N questions`, with each question and its answer on
/// the gutter.
pub(super) fn ask(tool: Tool<'_>, columns: usize) -> Card {
    let Some(result) = tool.result else {
        return Card {
            rows: Vec::new(),
            hidden: false,
        };
    };
    let request = Request::parse("", tool.arguments).ok();
    let error = result.get("error").and_then(crate::json::Value::as_str);
    let reply = Reply::parse(result);
    let status = match (&reply, error) {
        (_, Some(_)) => Status::Failed,
        (Some(Reply::Answered(_)), None) => Status::Done,
        _ => Status::Stopped,
    };
    let mut head = super::badge::badge("Ask user", status);
    head.push(" ", BODY);
    let subject = request.as_ref().map_or("Question".into(), Request::subject);
    head.push(&text::clean(&subject).replace('\n', " "), BODY);
    let mut rows = if tool.presentation.expanded {
        super::title::rows("Ask user", &subject, status, columns, true)
    } else {
        vec![head.shortened(columns)]
    };
    let items: Vec<Vec<(String, Style)>> = match (reply.as_ref(), &request) {
        (Some(Reply::Answered(answers)), Some(request)) => {
            let several = request.questions.len() > 1;
            request
                .questions
                .iter()
                .zip(answers)
                .map(|(question, answer)| {
                    let mut item = Vec::new();
                    if several {
                        item.push((format!("{}  ", question.title), MUTED));
                    }
                    item.push((answer_text(question, answer), STRONG));
                    item
                })
                .collect()
        }
        _ => vec![vec![(
            match (
                error,
                result.get("outcome").and_then(crate::json::Value::as_str),
            ) {
                (Some(error), _) => error.into(),
                (None, Some("dismissed")) => "No answer supplied".into(),
                (None, Some("cancelled")) => "Question cancelled · no answer supplied".into(),
                (None, Some("unavailable")) => "Interactive input unavailable".into(),
                _ => tool.summary.into(),
            },
            if error.is_some() { BAD } else { MUTED },
        )]],
    };
    if tool.presentation.expanded {
        let mut details = Vec::new();
        if let Some(request) = &request {
            for (index, question) in request.questions.iter().enumerate() {
                details.push(vec![(
                    format!("{}: {}", question.title, question.question),
                    BODY,
                )]);
                if !question.context.is_empty() {
                    details.push(vec![(question.context.clone(), MUTED)]);
                }
                for option in &question.options {
                    let mut choice = option.label.clone();
                    if !option.description.is_empty() {
                        choice.push_str(" · ");
                        choice.push_str(&option.description);
                    }
                    details.push(vec![(choice, MUTED)]);
                }
                if let Some(Reply::Answered(answers)) = &reply
                    && let Some(answer) = answers.get(index)
                {
                    details.push(vec![
                        ("Answer · ".into(), MUTED),
                        (answer_text(question, answer), STRONG),
                    ]);
                }
            }
        }
        if details.is_empty() || !matches!(reply, Some(Reply::Answered(_))) {
            details.extend(items);
        }
        let count = details.len();
        for (index, item) in details.into_iter().enumerate() {
            tree(&mut rows, item, index + 1 == count, columns);
        }
        return Card {
            rows,
            hidden: false,
        };
    }
    let count = items.len();
    for (index, item) in items.into_iter().enumerate() {
        tree(&mut rows, item, index + 1 == count, columns);
    }
    Card { rows, hidden: true }
}

/// The chosen labels, then the typed words after ` · `.
fn answer_text(question: &Question, answer: &Answer) -> String {
    let labels: Vec<&str> = answer
        .choices
        .iter()
        .filter_map(|id| {
            question
                .options
                .iter()
                .find(|option| &option.id == id)
                .map(|option| option.label.as_str())
        })
        .collect();
    [labels.join(", "), answer.text.clone()]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// One gutter item: `│` or `└` on its first row, wrapped two cells before
/// the edge; its own line breaks start new rows.
fn tree(rows: &mut Vec<Line>, item: Vec<(String, Style)>, last: bool, columns: usize) {
    let mut lines = vec![Line::default()];
    for (value, style) in item {
        for (index, part) in text::clean(&value).split('\n').enumerate() {
            if index > 0 {
                lines.push(Line::default());
            }
            lines.last_mut().unwrap().push(part, style);
        }
    }
    let parts = lines
        .iter()
        .flat_map(|line| line.wrap(columns.saturating_sub(4).max(1), true));
    for (index, part) in parts.enumerate() {
        let gutter = match (index, last) {
            (0, true) => "  └ ",
            (_, false) => "  │ ",
            (_, true) => "    ",
        };
        rows.push(part.gutter(gutter, DIM).shortened(columns));
    }
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
    if tool.presentation.expanded {
        let tasks = tool
            .result
            .and_then(|result| result.get("tasks"))
            .or_else(|| tool.arguments.get("tasks"))
            .and_then(crate::json::Value::as_array)
            .unwrap_or(&[]);
        let title = tool
            .result
            .map(|result| super::field(result, "title"))
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| super::field(tool.arguments, "title"));
        let mut rows = super::title::rows("Todo", title, status, columns, true);
        let count = Line::new(&text::counted(tasks.len(), "task"), MUTED);
        rows.extend(super::line_rows(&count, columns, 1, "  ⎿ ", "    ", DIM));
        for (index, task) in tasks.iter().enumerate() {
            let status = super::field(task, "status");
            let title = super::field(task, "title");
            let note = super::field(task, "note");
            let agent = super::field(task, "agent");
            let mut item = vec![(format!("{status} · {title}"), BODY)];
            if !note.is_empty() {
                item.push((format!(" · {note}"), MUTED));
            }
            if !agent.is_empty() {
                item.push((format!(" · {agent}"), MUTED));
            }
            tree(&mut rows, item, index + 1 == tasks.len(), columns);
        }
        if status == super::Status::Failed {
            rows.extend(super::failure::rows(&tool, columns, "  ⎿ "));
        }
        super::badge::hang(&mut rows[1..], true);
        return Card {
            rows,
            hidden: false,
        };
    }
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
        hidden: tool.result.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{json::Value, tui::cards::Presentation};

    fn open() -> Presentation {
        Presentation {
            expanded: true,
            ..Presentation::default()
        }
    }

    fn text(card: &Card) -> String {
        card.rows
            .iter()
            .map(Line::plain)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn expanded_question_shows_full_prompt_choices_and_answer_once() {
        let arguments = Value::object([(
            "questions",
            Value::Array(vec![Value::object([
                ("id", Value::string("destination")),
                (
                    "title",
                    Value::string("Choose the destination for the exported report"),
                ),
                (
                    "question",
                    Value::string("Where should the report be saved?"),
                ),
                (
                    "context",
                    Value::string("The chosen path will appear in the final receipt."),
                ),
                (
                    "options",
                    Value::Array(vec![Value::object([
                        ("id", Value::string("local")),
                        ("label", Value::string("Local folder")),
                        ("description", Value::string("Save beside the project")),
                    ])]),
                ),
            ])]),
        )]);
        let result = Value::object([
            ("outcome", Value::string("answered")),
            (
                "answers",
                Value::Array(vec![Value::object([
                    ("id", Value::string("destination")),
                    ("choices", Value::Array(vec![Value::string("local")])),
                    ("text", Value::string("reports/weekly.md")),
                ])]),
            ),
        ]);
        let shown = open();
        let card = super::super::card(
            Tool {
                name: "ask_user",
                arguments: &arguments,
                summary: "",
                result: Some(&result),
                change: None,
                presentation: &shown,
            },
            28,
            true,
        );
        let full = text(&card);
        for part in [
            "exported report",
            "saved?",
            "receipt.",
            "project",
            "reports/weekly.md",
        ] {
            assert!(full.contains(part), "missing {part}: {full}");
        }
        assert_eq!(full.matches("Ctrl+O collapse").count(), 1);
        assert!(!full.contains('…'));
        assert!(
            card.rows
                .iter()
                .all(|row| crate::tui::text::cells(&row.plain()) <= 28)
        );
    }

    #[test]
    fn expanded_todo_shows_complete_task_titles_and_notes() {
        let arguments = Value::object([
            (
                "title",
                Value::string("Prepare the release checklist for the maintainer"),
            ),
            (
                "tasks",
                Value::Array(vec![Value::object([
                    ("id", Value::string("verify")),
                    (
                        "title",
                        Value::string(
                            "Run checks across all supported platforms before publishing",
                        ),
                    ),
                    ("status", Value::string("in_progress")),
                    (
                        "note",
                        Value::string("Windows completed; Linux still queued"),
                    ),
                ])]),
            ),
        ]);
        let result = Value::object([("revision", Value::number(1))]);
        let shown = open();
        let card = super::super::card(
            Tool {
                name: "todo_list",
                arguments: &arguments,
                summary: "",
                result: Some(&result),
                change: None,
                presentation: &shown,
            },
            26,
            true,
        );
        let full = text(&card);
        for part in ["maintainer", "1 task", "publishing", "queued"] {
            assert!(full.contains(part), "missing {part}: {full}");
        }
        assert_eq!(full.matches("Ctrl+O collapse").count(), 1);
        assert!(!full.contains('…'));
        assert!(
            card.rows
                .iter()
                .all(|row| crate::tui::text::cells(&row.plain()) <= 26)
        );
    }
}
