//! Routine work: the badge and its purpose, the command on the tree, then
//! one quiet result.
use super::{
    Card, Status, Tool, badge, content, failure, field, heading, hint, line_rows, summary,
};
use crate::tui::{
    line::Line,
    text,
    theme::{DIM, Style},
};

pub(super) fn description<'a>(tool: &Tool<'a>) -> &'a str {
    field(tool.arguments, "description").trim()
}

pub(super) fn inline(value: &str) -> String {
    text::clean(value)
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn payload(tool: &Tool<'_>) -> String {
    let args = tool.arguments;
    let value = match tool.name {
        "bash" => field(args, "command"),
        "process" => tool
            .result
            .map(|result| field(result, "command"))
            .filter(|command| !command.is_empty())
            .unwrap_or_else(|| field(args, "command")),
        "browser" => match field(args, "action") {
            "evaluate" => field(args, "expression"),
            "open" | "navigate" => field(args, "url"),
            "click" | "fill" | "type" | "wait" => field(args, "selector"),
            "press" => field(args, "key"),
            "cdp" => field(args, "method"),
            _ => field(args, "tab_id"),
        },
        _ => return heading(tool).1,
    };
    if value.is_empty() && tool.name == "process" {
        format!("{} {}", field(args, "action"), field(args, "process_id"))
            .trim()
            .into()
    } else if tool.presentation.expanded {
        text::clean(value)
    } else {
        inline(value)
    }
}

pub(super) fn code(tool: &Tool<'_>) -> Line {
    Line::new(&payload(tool), DIM)
}

pub(super) fn one(line: Line, columns: usize, prefix: &str, style: Style, source: usize) -> Line {
    line.at_source(source)
        .shortened(columns.saturating_sub(text::cells(prefix)))
        .gutter(prefix, style)
        .shortened(columns)
}

pub(super) fn limited(
    line: &Line,
    columns: usize,
    prefix: &str,
    limit: usize,
    source: usize,
) -> Vec<Line> {
    let mut rows = line_rows(line, columns, source, prefix, "    ", DIM);
    if rows.len() > limit {
        rows.truncate(limit);
        let last = rows.last_mut().unwrap();
        last.push("…", DIM);
        *last = last.shortened(columns);
    }
    rows
}

/// ` NAME ` and the description or target in DIM; commands and discovery
/// targets follow on the tree when a description is supplied. Returns the
/// rows, how many belong to the title, and the gutter the result row takes.
fn title(tool: &Tool<'_>, columns: usize, status: Status) -> (Vec<Line>, usize, &'static str) {
    let (name, target) = heading(tool);
    let command_tool = matches!(tool.name, "bash" | "exec" | "process");
    let discovery_note = matches!(tool.name, "list" | "search") && !description(tool).is_empty();
    // Actions without a payload, such as snapshot or stop, have no command row.
    let command = if discovery_note {
        true
    } else if tool.name == "browser" {
        !payload(tool).is_empty()
    } else {
        command_tool && !description(tool).is_empty()
    };
    let mut head = badge::badge(name, status);
    head.push(" ", DIM);
    let display_target = if (command_tool && command) || discovery_note {
        description(tool)
    } else if tool.name == "browser" {
        field(tool.arguments, "action")
    } else {
        &target
    };
    head.push(&inline(display_target), DIM);
    // A call still running in the background keeps its trailing ` &`.
    if status == Status::Background {
        head.push(" &", DIM);
    }
    let mut rows = if tool.presentation.expanded {
        let mut wrapped = head.at_source(0).into_wrapped(columns.max(1), true);
        let mut rows: Vec<_> = wrapped.next().into_iter().collect();
        wrapped.set_columns(columns.saturating_sub(2).max(1));
        rows.extend(wrapped.map(|row| row.indent("  ", DIM)));
        rows
    } else {
        vec![head.at_source(0).shortened(columns)]
    };
    let titled = rows.len();
    if command {
        let command = if discovery_note {
            Line::new(&inline(&target), DIM)
        } else {
            Line::new(
                if tool.name == "exec" {
                    "> "
                } else if command_tool {
                    "$ "
                } else {
                    ""
                },
                DIM,
            )
            .append(code(tool))
        };
        rows.extend(if tool.presentation.expanded {
            line_rows(&command, columns, 1, "  ⎿ ", "    ", DIM)
        } else {
            limited(&command, columns, "  ⎿ ", 2, 1)
        });
    }
    (rows, titled, if command { "    " } else { "  ⎿ " })
}

pub(super) fn card(tool: Tool<'_>, columns: usize, keycap: bool) -> Card {
    let status = Status::of(&tool);
    let (rows, titled, guide) = title(&tool, columns, status);
    let mut card = if tool.presentation.expanded {
        expanded_card(tool, columns, status, rows)
    } else {
        collapsed_card(tool, columns, keycap, status, rows, guide)
    };
    badge::hang(&mut card.rows[titled..], true);
    card
}

fn collapsed_card(
    tool: Tool<'_>,
    columns: usize,
    keycap: bool,
    status: Status,
    mut rows: Vec<Line>,
    guide: &str,
) -> Card {
    if status == Status::Failed {
        rows.extend(failure::rows(&tool, columns, guide));
        let hidden = tool.result.is_some();
        if keycap && let Some(last) = rows.pop() {
            rows.extend(hint(last, "expand", columns));
        }
        rows.extend(tool.result.map_or_else(Vec::new, |result| {
            content::notices(tool.name, result, columns, false)
        }));
        return Card { rows, hidden };
    }
    let mut body = tool
        .result
        .filter(|result| result.get("error").is_none())
        .map(|result| content::render(&tool, result, status, columns, false));
    let mut lead = summary::line(&tool, status, None);
    let hidden = body.as_ref().is_some_and(|body| body.hidden > 0)
        || matches!(tool.name, "list" | "search") && tool.result.is_some();
    let mut suffix = Vec::new();
    if let Some(body) = &body
        && body.hidden > 0
    {
        suffix.push(format!("+{} {}", body.hidden, super::plural(body.hidden)));
    }
    if !suffix.is_empty() {
        let suffix = Line::new(&format!("  {}", suffix.join(" · ")), DIM);
        lead = lead
            .shortened(columns.saturating_sub(4 + text::cells(&suffix.plain())))
            .append(suffix);
    }
    let mut result_rows = vec![one(lead, columns, guide, DIM, 2)];
    if keycap
        && hidden
        && let Some(last) = result_rows.pop()
    {
        result_rows.extend(hint(last, "expand", columns));
    }
    rows.extend(result_rows);
    if let Some(body) = &mut body {
        rows.append(&mut body.rows);
    }
    if let Some(result) = tool.result {
        rows.extend(content::notices(tool.name, result, columns, false));
    }
    Card { rows, hidden }
}

/// Expanded output is a plain retained stream followed by its closing facts.
fn expanded_card(tool: Tool<'_>, columns: usize, status: Status, mut rows: Vec<Line>) -> Card {
    if let Some(result) = tool.result {
        if let Some(reason) = crate::events::not_run(result) {
            rows.extend(line_rows(
                &Line::new(&text::clean(reason), DIM),
                columns,
                3,
                "    ",
                "    ",
                DIM,
            ));
        } else if let Some(error) = result.get("error").and_then(crate::json::Value::as_str) {
            rows.extend(line_rows(
                &Line::new(&text::clean(error), DIM),
                columns,
                3,
                "    ",
                "    ",
                DIM,
            ));
        } else if let Some(launch) = summary::persistent(&tool, result) {
            if status != Status::Failed {
                rows.extend(line_rows(&launch, columns, 2, "    ", "    ", DIM));
            }
        } else if let Some(root) = super::root::line(&tool, result) {
            rows.extend(line_rows(&root, columns, 2, "    ", "    ", DIM));
            rows.extend(content::render(&tool, result, status, columns, true).rows);
        } else {
            rows.extend(content::render(&tool, result, status, columns, true).rows);
        }
        rows.extend(content::notices(tool.name, result, columns, true));
        let raw = crate::events::not_run(result)
            .or_else(|| result.get("error").and_then(crate::json::Value::as_str));
        let mut footer = if let Some(raw) = raw {
            super::outcome::marker(raw, status)
        } else if matches!(status, Status::Failed | Status::Stopped | Status::Unknown)
            && (result.get("error").is_some()
                || !matches!(tool.name, "bash" | "exec" | "process")
                || status != Status::Failed
                || field(result, "launch") == "persistent")
        {
            summary::line(&tool, status, None)
        } else {
            Line::default()
        };
        if footer.spans.is_empty()
            && matches!(tool.name, "bash" | "exec" | "process")
            && field(result, "launch") != "persistent"
        {
            let code = result
                .get("exit_code")
                .filter(|code| **code != crate::json::Value::Null);
            if code.is_some() || status == Status::Failed {
                let code = code.map_or_else(|| "unknown".into(), crate::json::Value::encode);
                footer = super::outcome::line(&format!("exit {code}"), status);
            }
        }
        let timed_outcome = status == Status::Stopped
            && (super::flag(result, "cancelled")
                || super::flag(result, "timed_out")
                || crate::events::process_status(result) == Some("timed_out"));
        if let Some(elapsed) = tool.presentation.duration().filter(|_| !timed_outcome) {
            if !footer.spans.is_empty() {
                footer.push(" · ", DIM);
            }
            footer.push(&crate::tui::activity::duration(elapsed), DIM);
        }
        if !footer.spans.is_empty() {
            rows.extend(line_rows(
                &footer,
                columns,
                usize::MAX - 110,
                "    ",
                "    ",
                DIM,
            ));
        }
    } else {
        rows.push(one(
            summary::line(&tool, status, None),
            columns,
            "    ",
            DIM,
            2,
        ));
    }
    Card {
        rows,
        hidden: false,
    }
}
