//! Decode display errors while preserving the original result for expansion.
use super::{Tool, field, rhythm, streams};
use crate::{
    json::{self, Value},
    tui::{
        line::Line,
        text,
        theme::{BAD, MUTED},
    },
};

fn browser(error: &str) -> Option<(String, String)> {
    let raw = error.strip_prefix("Browser JavaScript error: ")?;
    let value = json::parse(raw).ok()?;
    let exception = value.get("exception");
    let description = exception
        .map(|exception| field(exception, "description"))
        .filter(|s| !s.is_empty())
        .or_else(|| exception.map(|exception| field(exception, "value")))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| field(&value, "text"));
    let mut lines = description.lines();
    let message = lines.next().unwrap_or("JavaScript error").to_owned();
    let mut location = lines
        .find(|line| line.trim_start().starts_with("at "))
        .unwrap_or("")
        .trim()
        .to_owned();
    if location.is_empty()
        && let Some(frame) = value
            .get("stackTrace")
            .and_then(|stack| stack.get("callFrames"))
            .and_then(Value::as_array)
            .and_then(|frames| frames.first())
    {
        let number = |key| frame.get(key).and_then(Value::as_usize).unwrap_or(0) + 1;
        location = format!(
            "at {}:{}:{}",
            field(frame, "url"),
            number("lineNumber"),
            number("columnNumber")
        );
    }
    Some((message, location))
}

pub(super) fn rows(tool: &Tool<'_>, columns: usize, guide: &str) -> Vec<Line> {
    let Some(result) = tool.result else {
        return Vec::new();
    };
    let entries = streams::entries(result, true);
    let error = field(result, "error");
    let position = entries.iter().position(|entry| {
        entry.text.trim_start().starts_with("--> ") || entry.text.trim_start().starts_with("at ")
    });
    let (message, location, skip) = if !error.is_empty() {
        let (message, location) = browser(error).unwrap_or_else(|| {
            let (message, location) = error.split_once("\n    at ").map_or(
                (error, String::new()),
                |(message, location)| {
                    (
                        message,
                        format!("at {}", location.lines().next().unwrap_or("")),
                    )
                },
            );
            (message.into(), location)
        });
        (message, location, 0)
    } else {
        let exit = result
            .get("exit_code")
            .filter(|code| **code != Value::Null)
            .map_or_else(|| "unknown".into(), Value::encode);
        let first = entries
            .first()
            .map(|entry| entry.text.as_str())
            .unwrap_or("");
        (
            format!(
                "Exit {exit}{}{}",
                if first.is_empty() { "" } else { " · " },
                first
            ),
            position
                .map(|index| {
                    entries[index]
                        .text
                        .trim()
                        .trim_start_matches("--> ")
                        .to_owned()
                })
                .unwrap_or_default(),
            usize::from(!first.is_empty()),
        )
    };
    let mut lead = Line::new(&text::clean(&message), BAD);
    if !location.is_empty() {
        lead.push(&format!(" · {}", text::clean(&location)), MUTED);
    }
    let mut rows = rhythm::limited(&lead, columns, guide, 2, 2);
    rows.extend(
        entries
            .iter()
            .enumerate()
            .skip(skip)
            .filter(|(index, entry)| Some(*index) != position && entry.stream == "stderr")
            .take(2)
            .enumerate()
            .map(|(index, (_, entry))| {
                rhythm::one(
                    Line::new(entry.text.trim(), BAD),
                    columns,
                    "    ",
                    MUTED,
                    3 + index,
                )
            }),
    );
    rows
}
