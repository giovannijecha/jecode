use super::{Status, Tool, content, diff, edit_fallback, field, flag, plural, skill_name, streams};
use crate::{
    file_change::Change,
    json::Value,
    tui::{
        activity,
        line::Line,
        text,
        theme::{BAD, BODY, DIM, GOOD, LIT, MUTED, STRONG, WARNING},
    },
};
use std::time::Duration;

/// What came back, as `Subject · detail`.
pub(super) fn line(tool: &Tool<'_>, status: Status, change: Option<&Change>) -> Line {
    let Some(result) = tool.result else {
        return running(tool);
    };
    if let Some(error) = result.get("error").and_then(Value::as_str) {
        return Line::new(&short_error(error), BAD);
    }
    let elapsed = tool.presentation.duration();
    let output =
        !field(result, "stdout").trim().is_empty() || !field(result, "stderr").trim().is_empty();
    let process = crate::events::process_status(result);
    if flag(result, "cancelled") || flag(result, "timed_out") || process == Some("timed_out") {
        let subject = if flag(result, "cancelled") {
            "Cancelled"
        } else {
            "Timed out"
        };
        let mut line = Line::new(
            &elapsed.map_or_else(
                || subject.to_owned(),
                |elapsed| format!("{subject} after {}", activity::duration(elapsed)),
            ),
            WARNING,
        );
        if output {
            line.push(" · partial output", MUTED);
        }
        return line;
    }
    if let Some(processes) = result.get("processes").and_then(Value::as_array) {
        let running = processes
            .iter()
            .filter(|item| crate::events::process_status(item) == Some("running"))
            .count();
        let noun = if processes.len() == 1 {
            "process"
        } else {
            "processes"
        };
        return Line::new(
            &format!("{} {noun} · {running} running", processes.len()),
            MUTED,
        );
    }
    if let Some((state, style)) = process.and_then(|process| match process {
        "running" => Some((" running", LIT)),
        "stopping" => Some((" stopping", WARNING)),
        "stopped" => Some((" stopped", WARNING)),
        "unknown" => Some((" status unknown", MUTED)),
        _ => None,
    }) {
        let mut line = Line::new(field(result, "process_id"), STRONG);
        line.push(state, style);
        let entries = streams::entries(result, false);
        if let Some(index) =
            streams::headline(&entries, false).filter(|_| !tool.presentation.expanded)
        {
            line.push(&format!(" · {}", entries[index].text), MUTED);
        }
        return line;
    }
    if let Some(change) = change {
        return changed(change);
    }
    if tool.name == "edit"
        && let Some(summary) = edit_fallback::summary(tool.arguments, result)
    {
        return Line::new(&summary, MUTED);
    }
    match tool.name {
        "bash" | "exec" | "process" => command(
            result,
            status == Status::Failed,
            elapsed,
            tool.presentation.expanded,
        ),
        "read" => {
            let read = read(tool.arguments, result);
            match skill_name(field(tool.arguments, "path")) {
                Some(_) => Line::new(&format!("Loaded skill · {read}"), MUTED),
                None => Line::new(&read, MUTED),
            }
        }
        "write" => {
            let count = content::payload(field(tool.arguments, "content")).len();
            Line::new(&format!("Wrote {count} {}", plural(count)), MUTED)
        }
        "web" => web(result, tool.summary),
        "browser" => browser(result, tool.summary),
        _ => Line::new(tool.summary, MUTED),
    }
}

fn running(tool: &Tool<'_>) -> Line {
    let mut line = if matches!(tool.name, "read" | "write" | "edit") {
        Line::default()
    } else {
        Line::new("Running", DIM)
    };
    if let Some(elapsed) = tool.presentation.duration() {
        if !line.spans.is_empty() {
            line.push(" · ", DIM);
        }
        line.push(&activity::duration(elapsed), DIM);
    }
    if let Some(note) = &tool.presentation.note {
        if !line.spans.is_empty() {
            line.push(" · ", MUTED);
        }
        line.push(&text::clean(note), MUTED);
    }
    line
}

/// A missing file path reads `Not found`; the title already names it.
fn short_error(error: &str) -> String {
    let error = text::clean(error);
    if error.starts_with("Could not resolve path ")
        && (error.contains("(os error 2)") || cfg!(windows) && error.contains("(os error 3)"))
    {
        "Not found".into()
    } else {
        error
    }
}

/// A collapsed command names its result line; expanded, the output follows
/// in full and the summary keeps the exit and duration.
fn command(result: &Value, failed: bool, elapsed: Option<Duration>, expanded: bool) -> Line {
    let entries = streams::entries(result, failed);
    let headline = streams::headline(&entries, failed)
        .filter(|_| !expanded)
        .map(|index| entries[index].text.as_str());
    let elapsed = elapsed
        .filter(|elapsed| *elapsed >= Duration::from_secs(1))
        .map(activity::duration);
    if failed {
        let code = result
            .get("exit_code")
            .filter(|value| **value != Value::Null)
            .map_or_else(|| "unknown".into(), Value::encode);
        let mut line = Line::new(&format!("Exit {code}"), BAD);
        match (headline, elapsed) {
            (Some(headline), _) => line.push(&format!(" · {headline}"), BODY),
            (None, Some(elapsed)) => line.push(&format!(" · {elapsed}"), MUTED),
            (None, None) => {}
        }
        return line;
    }
    match (headline, elapsed) {
        (Some(headline), elapsed) => {
            let mut line = Line::new(headline, BODY);
            if let Some(elapsed) = elapsed {
                line.push(&format!(" · {elapsed}"), MUTED);
            }
            line
        }
        (None, Some(elapsed)) => Line::new(&format!("Done in {elapsed}"), MUTED),
        (None, None) => Line::new("Done", MUTED),
    }
}

fn changed(change: &Change) -> Line {
    if change.created {
        return Line::new(
            &format!("New file · {} {}", change.added, plural(change.added)),
            MUTED,
        );
    }
    if change.added == 0 && change.removed == 0 {
        return Line::new(&diff::summary(change), MUTED);
    }
    let mut line = Line::default();
    if change.added > 0 {
        line.push(&format!("+{}", change.added), GOOD);
    }
    if change.removed > 0 {
        if change.added > 0 {
            line.push(" ", MUTED);
        }
        line.push(&format!("−{}", change.removed), BAD);
    }
    line.push(
        &format!(" {}", plural(change.added + change.removed)),
        MUTED,
    );
    line
}

pub(super) fn read(arguments: &Value, result: &Value) -> String {
    if let Some(view) = result.get("view").and_then(Value::as_str) {
        let mut summary = format!("Opened {}", if view == "pdf" { "PDF" } else { view });
        if let Some(attachment) = result.get("attachment") {
            if view == "image"
                && let (Some(width), Some(height)) = (
                    attachment.get("width").and_then(Value::as_usize),
                    attachment.get("height").and_then(Value::as_usize),
                )
            {
                summary.push_str(&format!(" · {width}×{height}"));
            }
            if view == "pdf"
                && let Some(pages) = attachment.get("pages").and_then(Value::as_usize)
            {
                summary.push_str(&format!(
                    " · {pages} {}",
                    if pages == 1 { "page" } else { "pages" }
                ));
            }
            if let Some(bytes) = attachment.get("size").and_then(Value::as_usize) {
                summary.push_str(&format!(" · {}", size(bytes)));
            }
        }
        return summary;
    }
    if result.get("attachment").is_some() && result.get("content").is_none() {
        return "Attachment available".into();
    }
    if arguments.get("byte_offset").is_some() || result.get("offset") == Some(&Value::Null) {
        let count = result
            .get("bytes_returned")
            .and_then(Value::as_usize)
            .unwrap_or(0);
        let start = result
            .get("byte_offset")
            .and_then(Value::as_usize)
            .unwrap_or(0);
        let mut summary = match (count, start) {
            (0, _) => "0 bytes".into(),
            (count, 0) => format!("{count} bytes"),
            (count, start) => format!("{count} bytes · from byte {start}"),
        };
        if let Some(next) = result.get("next_byte_offset").and_then(Value::as_usize) {
            summary.push_str(&format!(" · more from {next}"));
        }
        return summary;
    }
    let count = lines(result);
    let start = result
        .get("offset")
        .or_else(|| arguments.get("offset"))
        .and_then(Value::as_usize)
        .unwrap_or(1);
    let mut summary = match (count, start) {
        (0, _) => "0 lines".into(),
        (count, 0 | 1) => format!("{count} {}", plural(count)),
        (count, start) => format!("{count} {} · from line {start}", plural(count)),
    };
    if start > 1
        && let Some(total) = result.get("total_lines").and_then(Value::as_usize)
    {
        summary.push_str(&format!(" of {total}"));
    } else if flag(result, "partial_line") {
        if let Some(next) = result.get("next_byte_offset").and_then(Value::as_usize) {
            summary.push_str(&format!(" · continues at byte {next}"));
        }
    } else if let Some(next) = result.get("next_offset").and_then(Value::as_usize) {
        summary.push_str(&format!(" · more from {next}"));
    }
    summary
}

/// Preserve the continuation on a narrow row by dropping its redundant start.
pub(super) fn compact_read(mut line: Line, width: usize) -> Line {
    if text::cells(&line.plain()) > width {
        for span in &mut line.spans {
            let start = span
                .text
                .find(" · from line ")
                .or_else(|| span.text.find(" · from byte "));
            let continuation = span
                .text
                .find(" · more from ")
                .or_else(|| span.text.find(" · continues at byte "));
            if let (Some(start), Some(continuation)) = (start, continuation)
                && continuation > start
            {
                span.text.replace_range(start..continuation, "");
            }
        }
    }
    line.shortened(width)
}

pub(super) fn lines(result: &Value) -> usize {
    result
        .get("lines_returned")
        .and_then(Value::as_usize)
        .unwrap_or_else(|| content::payload(field(result, "content")).len())
}

fn pair(subject: &str, details: &[String]) -> Line {
    let mut line = Line::new(&text::clean(subject), STRONG);
    for detail in details.iter().filter(|detail| !detail.is_empty()) {
        line.push(&format!(" · {}", text::clean(detail)), MUTED);
    }
    line
}

fn web(result: &Value, summary: &str) -> Line {
    let Some(status) = result
        .get("status")
        .filter(|_| result.get("pages").is_none())
    else {
        return Line::new(summary, MUTED);
    };
    let url = Some(field(result, "final_url"))
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| field(result, "requested_url"));
    let host = host(url);
    let title = field(result, "title").trim();
    let code = format!("HTTP {}", status.encode());
    // The page title leads, then the host, then the status.
    let subject = [title, host, &code]
        .into_iter()
        .find(|subject| !subject.is_empty())
        .unwrap_or_default();
    let mut details = Vec::new();
    if subject != code
        && !status
            .as_usize()
            .is_some_and(|code| (200..300).contains(&code))
    {
        details.push(code.clone());
    }
    details.push(size(
        result.get("bytes").and_then(Value::as_usize).unwrap_or(0),
    ));
    if !title.is_empty() {
        details.push(host.into());
    }
    pair(subject, &details)
}

fn browser(result: &Value, summary: &str) -> Line {
    if let Some(path) = result.get("screenshot_path").and_then(Value::as_str) {
        let attachment = result.get("attachment");
        let measure = |key| {
            attachment
                .and_then(|value| value.get(key))
                .and_then(Value::as_usize)
        };
        let subject = match (measure("width"), measure("height")) {
            (Some(width), Some(height)) => format!("Screenshot {width}×{height}"),
            _ => "Screenshot".into(),
        };
        return pair(&subject, &[path.into()]);
    }
    if result.get("snapshot_path").is_some() {
        let title = field(result, "title").trim();
        return pair(
            if title.is_empty() { "Snapshot" } else { title },
            &[field(result, "url").into()],
        );
    }
    if let Some(url) = summary.strip_prefix("opened ") {
        return pair("Opened", &[url.into()]);
    }
    let mut characters = summary.chars();
    let subject = characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(characters).collect()
    });
    pair(&subject, &[])
}

fn host(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

/// Bytes in binary units, as capture limits are set.
pub(super) fn size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{} KiB", (bytes + 512) / 1024)
    } else {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}
