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
    let mut rows = vec![head.shortened(columns)];
    let items: Vec<Vec<(String, Style)>> = match (reply, &request) {
        (Some(Reply::Answered(answers)), Some(request)) => {
            let several = request.questions.len() > 1;
            request
                .questions
                .iter()
                .zip(&answers)
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
    let count = items.len();
    for (index, item) in items.into_iter().enumerate() {
        tree(&mut rows, item, index + 1 == count, columns);
    }
    Card {
        rows,
        hidden: false,
    }
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
