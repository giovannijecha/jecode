//! Routine work: the badge and its purpose, the command on the tree, then
//! one quiet result.
use super::{
    Card, Status, Tool, badge, content, failure, field, heading, hint, line_rows, summary,
};
use crate::tui::{
    line::Line,
    syntax::Highlighter,
    text,
    theme::{BAD, BODY, CODE_TEXT, DIM, LIT, MUTED, Style},
};

pub(super) fn description<'a>(tool: &Tool<'a>) -> &'a str {
    field(tool.arguments, "description").trim()
}

fn inline(value: &str) -> String {
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
            "click" | "fill" | "wait" => field(args, "selector"),
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
    } else {
        inline(value)
    }
}

pub(super) fn code(tool: &Tool<'_>) -> Line {
    let mut line = if tool.name == "exec" {
        Line::new(&payload(tool), CODE_TEXT)
    } else {
        Highlighter::new(if tool.name == "browser" { "js" } else { "sh" }).line(&payload(tool))
    };
    line.background = None;
    for span in &mut line.spans {
        span.background = None;
    }
    line
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
        last.push("…", MUTED);
        *last = last.shortened(columns);
    }
    rows
}

/// ` NAME ` and the description or target in BODY; commands and discovery
/// targets follow on the tree when a description is supplied. Returns the
/// rows, how many belong to the title, and the gutter the result row takes.
fn title(tool: &Tool<'_>, columns: usize, status: Status) -> (Vec<Line>, usize, &'static str) {
    let (name, target) = heading(tool);
    let command_tool = matches!(tool.name, "bash" | "exec" | "process");
    let discovery_note = (matches!(tool.name, "list" | "search")
        || tool.name == "map" && !matches!(field(tool.arguments, "action"), "remember" | "forget"))
        && !description(tool).is_empty();
    // Actions without a payload, such as snapshot or stop, have no command row.
    let command = if discovery_note {
        true
    } else if tool.name == "browser" {
        !payload(tool).is_empty()
    } else {
        command_tool && !description(tool).is_empty()
    };
    let mut head = badge::badge(name, status);
    head.push(" ", BODY);
    let display_target = if (command_tool && command) || discovery_note {
        description(tool)
    } else if tool.name == "browser" {
        field(tool.arguments, "action")
    } else {
        &target
    };
    head.push(&inline(display_target), BODY);
    // A call still running in the background ends its target in a bold ` &`.
    if status == Status::Background {
        head.push(" &", LIT.bold());
    }
    let mut rows = if tool.presentation.expanded && !command {
        let mut wrapped = head.at_source(0).into_wrapped(columns.max(1), true);
        let mut rows: Vec<_> = wrapped.next().into_iter().collect();
        wrapped.set_columns(columns.saturating_sub(2).max(1));
        rows.extend(wrapped.map(|row| row.indent("  ", BODY)));
        rows
    } else {
        vec![head.at_source(0).shortened(columns)]
    };
    let titled = rows.len();
    if command {
        let command = if discovery_note {
            Line::new(&inline(&target), CODE_TEXT)
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
    for span in &mut lead.spans {
        // Running and background facts stay quiet; the badge carries the state.
        span.style = if matches!(status, Status::Running | Status::Background) {
            DIM
        } else {
            MUTED
        };
    }
    let hidden = body.as_ref().is_some_and(|body| body.hidden > 0)
        || matches!(tool.name, "list" | "search" | "map") && tool.result.is_some();
    let mut suffix = Vec::new();
    if let Some(body) = &body
        && body.hidden > 0
    {
        suffix.push(format!("+{} {}", body.hidden, super::plural(body.hidden)));
    }
    if tool
        .result
        .and_then(|result| result.get("checkpoint"))
        .is_some_and(|checkpoint| field(checkpoint, "status") == "none")
    {
        suffix.push("no checkpoint".into());
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
        if let Some(error) = result.get("error").and_then(crate::json::Value::as_str) {
            rows.extend(line_rows(
                &Line::new(&text::clean(error), BAD),
                columns,
                3,
                "    ",
                "    ",
                DIM,
            ));
        } else {
            rows.extend(content::render(&tool, result, status, columns, true).rows);
        }
        rows.extend(content::notices(tool.name, result, columns, true));
        let mut footer = Line::default();
        if matches!(tool.name, "bash" | "exec" | "process")
            && let Some(code) = result
                .get("exit_code")
                .filter(|code| **code != crate::json::Value::Null)
        {
            footer.push(&format!("exit {}", code.encode()), DIM);
        }
        if let Some(elapsed) = tool.presentation.duration() {
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
