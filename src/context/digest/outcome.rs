//! One exact line per tool call: what was done and how it ended.

use crate::{
    context::view::excerpt,
    json::{self, Value},
};

const LINE_LIMIT: usize = 360;
const TAIL_LINES: usize = 8;
const TAIL_BYTES: usize = 240;
const FAILURE_BYTES: usize = 480;
const CHANGE_BYTES: usize = 80;
const QUESTION_BYTES: usize = 240;
const ANSWER_BYTES: usize = 4096;

pub(super) struct Outcome {
    pub line: String,
    pub failed: bool,
    /// Paths this call changed.
    pub changed: Vec<String>,
    /// A command, whose last result says where the work stood.
    pub command: bool,
    /// The user's answer to a question, which outlasts condensing.
    pub decision: bool,
}

pub(super) fn read(name: &str, arguments: &Value, result: &Value) -> Vec<Outcome> {
    let name = name.strip_prefix("jecode.").unwrap_or(name);
    if name == "steps"
        && let Some(steps) = result.get("steps").and_then(Value::as_array)
    {
        let calls = arguments.get("steps").and_then(Value::as_array);
        return steps
            .iter()
            .enumerate()
            .flat_map(|(index, step)| {
                let call = calls.and_then(|calls| calls.get(index));
                let tool = step
                    .get("tool")
                    .or_else(|| call.and_then(|call| call.get("tool")))
                    .and_then(Value::as_str)
                    .unwrap_or("step");
                let arguments = call
                    .and_then(|call| call.get("arguments"))
                    .map_or(Value::Null, decoded);
                read(tool, &arguments, step.get("result").unwrap_or(step))
            })
            .collect();
    }
    if name == "ask_user" {
        let mut outcome = asking(arguments, result);
        finish(&mut outcome, name, result);
        return vec![outcome];
    }
    // Denied, unavailable or never run: what the arguments describe did not happen.
    if let Some(state) = text(result, "outcome").filter(|state| *state != "completed") {
        let call = text(arguments, "path").map_or_else(|| call_text(arguments), str::to_owned);
        let mut outcome = plain(format!("{name} {call} → {state}"));
        outcome.failed = true;
        finish(&mut outcome, name, result);
        return vec![outcome];
    }
    let mut outcome = match name {
        "read" => reading(arguments, result),
        "write" => writing(arguments, result),
        "edit" => editing(arguments, result),
        "search" => searching(arguments, result),
        "list" => plain(format!(
            "list {} → {} entries",
            text(arguments, "path").unwrap_or("."),
            number(result, "entries_returned")
        )),
        "todo_list" => plain(format!(
            "todo_list {} ({} tasks)",
            text(result, "title").unwrap_or(""),
            result
                .get("tasks")
                .and_then(Value::as_array)
                .map_or(0, <[Value]>::len)
        )),
        _ => other(name, arguments, result),
    };
    finish(&mut outcome, name, result);
    vec![outcome]
}

fn plain(line: String) -> Outcome {
    Outcome {
        line,
        failed: false,
        changed: Vec::new(),
        command: false,
        decision: false,
    }
}

/// Failures are what a later step most needs, so they keep their text.
fn finish(outcome: &mut Outcome, name: &str, result: &Value) {
    if let Some(error) = text(result, "error") {
        outcome.failed = true;
        outcome.changed.clear();
        outcome.line = format!(
            "{} → error: {}",
            one_line(&outcome.line),
            one_line(&excerpt(error, FAILURE_BYTES))
        );
        return;
    }
    for flag in ["cancelled", "timed_out"] {
        if result.get(flag) == Some(&Value::Bool(true)) {
            outcome.failed = true;
            outcome.line.push_str(&format!(" → {flag}"));
        }
    }
    // A command still running has not failed; the same rule as for stopping steps
    // otherwise, which also knows the failures tools report through their status.
    let running = ["yielded", "moved_to_background"]
        .iter()
        .any(|flag| result.get(flag) == Some(&Value::Bool(true)));
    // Questions use their own successful outcome and cannot run inside steps.
    let answered = name == "ask_user" && text(result, "outcome") == Some("answered");
    outcome.failed |= !running && !answered && crate::agent::steps::failed(name, result);
    outcome.line = one_line(&outcome.line);
    if !outcome.changed.is_empty() {
        return;
    }
    let limit = if outcome.decision {
        QUESTION_BYTES + ANSWER_BYTES + FAILURE_BYTES
    } else {
        LINE_LIMIT + FAILURE_BYTES
    };
    outcome.line = excerpt(&outcome.line, limit);
}

fn decoded(value: &Value) -> Value {
    value
        .as_str()
        .and_then(|text| json::parse(text).ok())
        .unwrap_or_else(|| value.clone())
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

fn number(value: &Value, key: &str) -> usize {
    value.get(key).and_then(Value::as_usize).unwrap_or(0)
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn reading(arguments: &Value, result: &Value) -> Outcome {
    let file_failed = |file: &&Value| {
        file.get("error").is_some()
            || file
                .get("outcome")
                .is_some_and(|outcome| outcome.as_str() != Some("completed"))
    };
    let part = |path: &str, file: &Value| {
        if let Some(error) = text(file, "error") {
            return format!("{path} (error: {})", one_line(&excerpt(error, 200)));
        }
        if let Some(state) = text(file, "outcome").filter(|state| *state != "completed") {
            return format!("{path} ({state})");
        }
        // A byte-range page numbers no lines and never reports a total.
        if matches!(file.get("offset"), Some(Value::Null))
            && let Some(start) = file.get("byte_offset").and_then(Value::as_usize)
        {
            let end = start + number(file, "bytes_returned");
            let more = file.get("truncated") == Some(&Value::Bool(true));
            return format!(
                "{path} (bytes {start}-{end}{})",
                if more { ", more remain" } else { "" }
            );
        }
        let returned = number(file, "lines_returned");
        let offset = number(file, "offset").max(1);
        let end = (offset + returned).saturating_sub(1);
        // A page that stops before the end of the file reports no total.
        match file.get("total_lines").and_then(Value::as_usize) {
            _ if returned == 0 => path.to_owned(),
            Some(total) if returned >= total => format!("{path} ({returned} lines)"),
            Some(total) => format!("{path} (lines {offset}-{end} of {total})"),
            None => format!("{path} (lines {offset}-{end}, more remain)"),
        }
    };
    let files = result.get("files").and_then(Value::as_array);
    let failed = files.is_some_and(|files| files.iter().any(|file| file_failed(&file)));
    let line = match files {
        Some(files) => files
            .iter()
            .filter(file_failed)
            .chain(files.iter().filter(|file| !file_failed(file)))
            .map(|file| part(text(file, "path").unwrap_or("?"), file))
            .collect::<Vec<_>>()
            .join(", "),
        None => part(text(arguments, "path").unwrap_or("?"), result),
    };
    Outcome {
        failed,
        ..plain(format!("read {line}"))
    }
}

/// What was asked and what the user said, kept like a request.
fn asking(arguments: &Value, result: &Value) -> Outcome {
    let questions = arguments.get("questions").map(decoded);
    let asked = questions
        .as_ref()
        .and_then(Value::as_array)
        .map(|questions| {
            questions
                .iter()
                .filter_map(|question| text(question, "question"))
                .collect::<Vec<_>>()
                .join(" / ")
        })
        .unwrap_or_default();
    let reply = match result.get("answers").and_then(Value::as_array) {
        Some(answers) => answers
            .iter()
            .map(|answer| {
                let choices = answer
                    .get("choices")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str);
                choices
                    .chain(text(answer, "text"))
                    .collect::<Vec<_>>()
                    .join(": ")
            })
            .collect::<Vec<_>>()
            .join(" / "),
        None => text(result, "outcome").unwrap_or("no answer").to_owned(),
    };
    Outcome {
        line: format!(
            "ask_user {} → {}",
            one_line(&excerpt(&asked, QUESTION_BYTES)),
            one_line(&excerpt(&reply, ANSWER_BYTES))
        ),
        decision: true,
        ..plain(String::new())
    }
}

fn writing(arguments: &Value, result: &Value) -> Outcome {
    let path = text(arguments, "path").unwrap_or("?").to_owned();
    Outcome {
        line: format!("write {path} ({} bytes)", number(result, "bytes_written")),
        changed: vec![path],
        ..plain(String::new())
    }
}

fn editing(arguments: &Value, result: &Value) -> Outcome {
    // An empty placeholder runs as a single-file edit, as the tool reads it.
    let files = arguments
        .get("files")
        .map(decoded)
        .filter(|files| !crate::tools::edit::placeholder(files));
    let groups = match files.as_ref().and_then(Value::as_array) {
        Some(files) => files.iter().collect::<Vec<_>>(),
        None => vec![arguments],
    };
    // Keep failures first so excerpts cannot hide the first failed file behind
    // successful edits. A failed file did not apply its requested replacements.
    let mut parts = result
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|file| {
            text(file, "error").map(|error| {
                format!(
                    "{} failed: {}",
                    text(file, "path").unwrap_or("?"),
                    one_line(&excerpt(error, 200))
                )
            })
        })
        .collect::<Vec<_>>();
    let mut changed = Vec::new();
    for (index, group) in groups.into_iter().enumerate() {
        let path = text(group, "path").unwrap_or("?");
        let receipt = result
            .get("files")
            .and_then(Value::as_array)
            .and_then(|files| files.get(index))
            .unwrap_or(result);
        if receipt.get("error").is_some() {
            continue;
        }
        if receipt.get("changed") == Some(&Value::Bool(false)) {
            parts.push(format!("{path} (unchanged)"));
            continue;
        }
        let edits = group
            .get("edits")
            .map(decoded)
            .filter(|edits| !crate::tools::edit::placeholder(edits));
        let (added, removed) = match edits.as_ref().and_then(Value::as_array) {
            Some(edits) => edits
                .iter()
                .map(line_counts)
                .fold((0, 0), |sum, one| (sum.0 + one.0, sum.1 + one.1)),
            None => line_counts(group),
        };
        let first = match edits.as_ref().and_then(Value::as_array) {
            Some(edits) => edits.first().and_then(first_change),
            None => first_change(group),
        };
        parts.push(match first {
            Some(line) => format!("{path} (+{added} -{removed}): {line}"),
            None => format!("{path} (+{added} -{removed})"),
        });
        changed.push(path.to_owned());
    }
    Outcome {
        line: format!("edit {}", parts.join(", ")),
        changed,
        ..plain(String::new())
    }
}

/// The first line an edit added: a cheap anchor for what changed and where.
fn first_change(edit: &Value) -> Option<String> {
    let old = text(edit, "old_text").unwrap_or("");
    let line = text(edit, "new_text")?
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !old.contains(line))?;
    Some(line[..line.floor_char_boundary(CHANGE_BYTES)].to_owned())
}

fn line_counts(edit: &Value) -> (usize, usize) {
    let lines = |key| text(edit, key).map_or(0, |text| text.lines().count());
    (lines("new_text"), lines("old_text"))
}

fn searching(arguments: &Value, result: &Value) -> Outcome {
    let places = result
        .get("matches")
        .and_then(Value::as_array)
        .map(|matches| {
            matches
                .iter()
                .take(6)
                .map(|found| {
                    format!(
                        "{}:{}",
                        text(found, "path").unwrap_or("?"),
                        number(found, "line")
                    )
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    plain(format!(
        "search {:?} in {} → {} matches {places}",
        text(arguments, "query").unwrap_or(""),
        text(arguments, "path").unwrap_or("."),
        number(result, "matches_returned"),
    ))
}

/// A call's command, or its arguments without the progress label.
fn call_text(arguments: &Value) -> String {
    let mut shown = arguments.clone();
    if let Value::Object(fields) = &mut shown {
        fields.remove("work");
    }
    let call = text(arguments, "command").map_or_else(|| shown.encode(), str::to_owned);
    excerpt(&call, LINE_LIMIT / 2)
}

/// Everything else, with commands recognized by the exit code they report.
fn other(name: &str, arguments: &Value, result: &Value) -> Outcome {
    let call = call_text(arguments);
    let Some(exit) = result.get("exit_code").map(Value::encode) else {
        let answer = excerpt(&result.encode(), LINE_LIMIT / 2);
        return plain(format!("{name} {call} → {answer}"));
    };
    let stdout = text(result, "stdout").unwrap_or("");
    let stderr = text(result, "stderr").unwrap_or("");
    let failed = exit != "0" && exit != "null";
    let state = if exit == "null" {
        text(result, "status").unwrap_or("no exit code").to_owned()
    } else {
        format!("exit {exit}")
    };
    let tail = if failed {
        tail(&format!("{stdout}\n{stderr}"), FAILURE_BYTES)
    } else {
        tail(stdout, TAIL_BYTES)
    };
    let saved = if result.get("truncated") == Some(&Value::Bool(true)) {
        text(result, "stdout_file").map_or(String::new(), |file| format!(" (full: {file})"))
    } else {
        String::new()
    };
    Outcome {
        line: format!("{name} {call} → {state}{saved}: {tail}"),
        failed,
        command: true,
        ..plain(String::new())
    }
}

/// The last lines of a command's output, where runners report their totals.
fn tail(text: &str, limit: usize) -> String {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let joined = lines[lines.len().saturating_sub(TAIL_LINES)..].join(" | ");
    let start = joined.ceil_char_boundary(joined.len().saturating_sub(limit));
    joined[start..].to_owned()
}
