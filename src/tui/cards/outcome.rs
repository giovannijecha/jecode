//! A short state marker beside quiet facts or an unstructured diagnostic.
use super::Status;
use crate::tui::{
    line::Line,
    text,
    theme::{BAD, DIM, Style, WARNING},
};

fn ink(status: Status) -> Style {
    match status {
        Status::Failed => BAD,
        Status::Stopped | Status::Unknown => WARNING,
        _ => DIM,
    }
}

/// A structured outcome already has its marker before the first fact separator.
pub(super) fn line(value: &str, status: Status) -> Line {
    let value = text::clean(value);
    let end = value.find(" · ").unwrap_or(value.len());
    let mut line = Line::new(&value[..end], ink(status));
    line.push(&value[end..], DIM);
    line
}

fn heading(value: &str, status: Status) -> &str {
    let head = value.split(" · ").next().unwrap_or(value);
    match head {
        "Failed"
        | "Not run"
        | "Refused before dispatch"
        | "Denied"
        | "Not found"
        | "Server error"
        | "Server exited"
        | "Not restarted"
        | "Outcome unknown" => head,
        _ => match status {
            Status::Stopped => "Not run",
            Status::Unknown => "Outcome unknown",
            _ => "Failed",
        },
    }
}

/// Keep raw paths and explanations quiet, adding a marker only when absent.
pub(super) fn diagnostic(value: &str, status: Status) -> Line {
    if !matches!(status, Status::Failed | Status::Stopped | Status::Unknown) {
        return Line::new(&text::clean(value), DIM);
    }
    let value = text::clean(value);
    let head = heading(&value, status);
    if value.is_empty() {
        return Line::new(head, ink(status));
    }
    if value == head
        || value
            .strip_prefix(head)
            .is_some_and(|rest| rest.starts_with(" · "))
    {
        return line(&value, status);
    }
    let mut line = Line::new(head, ink(status));
    line.push(&format!(" · {value}"), DIM);
    line
}

/// The diagnostic is already in the expanded body; close with its marker only.
pub(super) fn marker(value: &str, status: Status) -> Line {
    let value = text::clean(value);
    Line::new(heading(&value, status), ink(status))
}
