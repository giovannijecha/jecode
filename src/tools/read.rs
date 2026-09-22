use super::{Output, number, string};
use crate::{
    json::{self, Value},
    workspace::{Budget, Error, Workspace},
};

pub(super) fn list(
    workspace: &Workspace,
    path: &str,
    limit: usize,
    budget: &Budget<'_>,
) -> Result<Output, Error> {
    let listing = workspace.list(path, budget)?;
    let total = listing.entries.len();
    let mut entries = Vec::new();
    let mut bytes = 0;
    for entry in listing.entries.into_iter().take(limit) {
        let value = json::object([
            ("name", string(&entry.name)),
            (
                "type",
                string(if entry.directory { "directory" } else { "file" }),
            ),
        ]);
        bytes += json::encode(&value, super::MAX_OUTPUT).map_or(super::MAX_OUTPUT, |s| s.len());
        if bytes > 24 * 1024 {
            break;
        }
        entries.push(value);
    }
    let count = entries.len();
    let truncated = listing.truncated || count < total;
    Ok(Output::success(
        json::object([
            ("ok", Value::Bool(true)),
            ("path", string(path)),
            ("entries", Value::Array(entries)),
            ("omitted", number(listing.omitted)),
            ("truncated", Value::Bool(truncated)),
        ]),
        format!(
            "{count} entries / {} omitted{}",
            listing.omitted,
            if truncated { " / limited" } else { "" }
        ),
        truncated || listing.omitted != 0,
    ))
}

pub(super) fn read(
    workspace: &Workspace,
    path: &str,
    start: usize,
    limit: usize,
    budget: &Budget<'_>,
) -> Result<Output, Error> {
    let source = workspace.read(path, budget)?;
    let mut lines = source.split_inclusive('\n').skip(start - 1).peekable();
    let mut text = String::new();
    let mut count = 0;
    while count < limit {
        let Some(line) = lines.peek() else { break };
        if text.len() + line.len() > 8192 {
            break;
        }
        text.push_str(line);
        count += 1;
        lines.next();
    }
    let truncated = lines.peek().is_some();
    if count == 0 && truncated {
        return Ok(Output::error(
            "requested line exceeds the 8 KiB output limit; no partial line returned",
        ));
    }
    budget.check()?;
    Ok(Output::success(
        json::object([
            ("ok", Value::Bool(true)),
            ("path", string(path)),
            ("start_line", number(start)),
            (
                "end_line",
                if count == 0 {
                    Value::Null
                } else {
                    number(start + count - 1)
                },
            ),
            ("text", string(&text)),
            ("truncated", Value::Bool(truncated)),
            (
                "next_line",
                if truncated {
                    number(start + count)
                } else {
                    Value::Null
                },
            ),
        ]),
        format!(
            "{count} lines{}",
            if truncated { " / more available" } else { "" }
        ),
        truncated,
    ))
}
