//! Direction 10: quiet routine work, with purpose above the command.
use super::{
    Card, Status, Tool, content, failure, field, heading, hint, line_rows, summary, title,
};
use crate::tui::{
    line::Line,
    syntax::Highlighter,
    text,
    theme::{BAD, BODY, MUTED, Style, TOOL_ACCENT, TOOL_DIM, TOOL_PULSE},
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
    let mut line =
        Highlighter::new(if tool.name == "browser" { "js" } else { "sh" }).line(&payload(tool));
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
    let mut rows = line_rows(line, columns, source, prefix, "    ", TOOL_DIM);
    if rows.len() > limit {
        rows.truncate(limit);
        let last = rows.last_mut().unwrap();
        last.push("…", MUTED);
        *last = last.shortened(columns);
    }
    rows
}

fn title(tool: &Tool<'_>, columns: usize, status: Status) -> (Vec<Line>, &'static str) {
    let (name, target) = heading(tool);
    let marker = if status == Status::Running {
        let elapsed = tool.presentation.duration().unwrap_or_default();
        TOOL_PULSE[((elapsed.as_millis() / 240) % TOOL_PULSE.len() as u128) as usize]
    } else {
        status.style()
    };
    let shell = matches!(tool.name, "bash" | "process");
    let command = tool.name == "browser" || shell && !description(tool).is_empty();
    let mut head = Line::new(name, title::family(name).bold());
    if shell && command {
        head.push("  ", BODY);
        head.push(&inline(description(tool)), BODY);
    } else {
        let target = if tool.name == "browser" {
            field(tool.arguments, "action")
        } else {
            &target
        };
        head.push("(", if tool.name == "browser" { MUTED } else { BODY });
        head.push(
            &inline(target),
            if tool.name == "browser" { MUTED } else { BODY },
        );
        head.push(")", if tool.name == "browser" { MUTED } else { BODY });
    }
    let mut rows = if tool.presentation.expanded && !command {
        line_rows(&head, columns, 0, "● ", "  ", marker)
    } else {
        vec![one(head, columns, "● ", marker, 0)]
    };
    if command {
        let mut command = Line::new(if shell { "$ " } else { "" }, MUTED).append(code(tool));
        if tool.name == "process" {
            command.push(" &", TOOL_ACCENT.bold());
        }
        rows.extend(if tool.presentation.expanded {
            line_rows(&command, columns, 1, "  ⎿ ", "    ", TOOL_DIM)
        } else {
            limited(&command, columns, "  ⎿ ", 2, 1)
        });
    }
    (rows, if command { "    " } else { "  ⎿ " })
}

pub(super) fn card(tool: Tool<'_>, columns: usize, keycap: bool) -> Card {
    let status = Status::of(&tool);
    let expanded = tool.presentation.expanded;
    let (mut rows, guide) = title(&tool, columns, status);
    if expanded {
        return expanded_card(tool, columns, status, rows);
    }
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
        span.style = if status == Status::Background {
            TOOL_ACCENT
        } else if status == Status::Running {
            TOOL_DIM
        } else {
            MUTED
        };
    }
    let hidden = body.as_ref().is_some_and(|body| body.hidden > 0);
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
        let suffix = Line::new(&format!("  {}", suffix.join(" · ")), TOOL_DIM);
        lead = lead
            .shortened(columns.saturating_sub(4 + text::cells(&suffix.plain())))
            .append(suffix);
    }
    let mut result_rows = vec![one(lead, columns, guide, TOOL_DIM, 2)];
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
                TOOL_DIM,
            ));
        } else {
            rows.extend(content::render(&tool, result, status, columns, true).rows);
        }
        rows.extend(content::notices(tool.name, result, columns, true));
        let mut footer = Line::default();
        if matches!(tool.name, "bash" | "process")
            && let Some(code) = result
                .get("exit_code")
                .filter(|code| **code != crate::json::Value::Null)
        {
            footer.push(&format!("exit {}", code.encode()), TOOL_DIM);
        }
        if let Some(elapsed) = tool.presentation.duration() {
            if !footer.spans.is_empty() {
                footer.push(" · ", TOOL_DIM);
            }
            footer.push(&crate::tui::activity::duration(elapsed), TOOL_DIM);
        }
        if !footer.spans.is_empty() {
            rows.extend(line_rows(
                &footer,
                columns,
                usize::MAX - 110,
                "    ",
                "    ",
                TOOL_DIM,
            ));
        }
    } else {
        rows.push(one(
            summary::line(&tool, status, None),
            columns,
            "    ",
            TOOL_DIM,
            2,
        ));
    }
    Card {
        rows,
        hidden: false,
    }
}
