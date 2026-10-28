//! Collaboration receipts and the read-only current-session panel.

use super::panel::{self, Field, Part, Sheet, key};
use crate::{
    json::Value,
    tui::{
        agents::{self, Panel},
        line::Line,
        state::State,
        text,
        theme::{ACCENT, BAD, BADGE_BAD, BADGE_IDLE, BADGE_WARN, DIM, LIT, MUTED, STRONG, WARNING},
    },
};

fn badge(label: &str, kind: &str) -> Line {
    let (style, surface) = match kind {
        "failed" => (STRONG, BADGE_BAD),
        "unknown" => (STRONG, BADGE_WARN),
        _ => (LIT, BADGE_IDLE),
    };
    let mut line = Line::default();
    line.push_on(&format!(" {label} "), style, surface);
    line
}

pub(in crate::tui) fn receipt(event: &Value, expanded: bool, columns: usize) -> Vec<Line> {
    let kind = agents::value(event, "kind");
    let label = if kind == "created" {
        "AGENTS".to_string()
    } else {
        format!("AGENT {}", agents::number(event, "number"))
    };
    let failed = kind == "returned"
        && agents::list(event, "records").iter().any(|record| {
            agents::value(record, "id") == agents::value(event, "id")
                && (agents::value(record, "state") == "failed"
                    || agents::status(record).starts_with("Failed"))
        });
    let state = if kind == "unknown" {
        "unknown"
    } else if failed {
        "failed"
    } else {
        ""
    };
    let mut head = Line::new("› ", ACCENT.bold()).append(badge(&label, state));
    head.push(&format!("  {}", agents::value(event, "title")), STRONG);
    let mut rows = head.wrap(columns.max(1), true);
    for row in rows.iter_mut().skip(1) {
        *row = std::mem::take(row).indent("  ", DIM).shortened(columns);
    }
    let summary = agents::value(event, "summary");
    if !summary.is_empty() {
        let version = agents::number(event, "version");
        let summary = if kind == "created" {
            summary.to_string()
        } else {
            format!("{summary} · v{version}")
        };
        rows.extend(
            Line::new(
                &summary,
                if state == "unknown" {
                    WARNING
                } else if failed {
                    BAD
                } else {
                    MUTED
                },
            )
            .wrap(columns.saturating_sub(4).max(1), true)
            .into_iter()
            .map(|line| line.indent("  ├ ", DIM)),
        );
    }
    if expanded {
        let records = if kind == "created" {
            agents::list(event, "records")
        } else {
            Vec::new()
        };
        for record in records {
            let mut row = Line::new("  ├ ", DIM).append(badge(&agents::label(record), ""));
            row.push(&format!("  {}", agents::value(record, "title")), STRONG);
            let mut wrapped = row.wrap(columns.max(1), true);
            for row in wrapped.iter_mut().skip(1) {
                *row = std::mem::take(row).indent("  │   ", DIM).shortened(columns);
            }
            rows.extend(wrapped);
            let workspace = agents::value(record, "workspace");
            let directory = agents::value(record, "directory");
            let version = agents::number(record, "received_version")
                .max(agents::number(record, "sent_version"));
            let fact = format!("Assignment v{version} · {workspace} · {directory}");
            rows.extend(
                Line::new(&fact, MUTED)
                    .wrap(columns.saturating_sub(6).max(1), true)
                    .into_iter()
                    .map(|line| line.indent("  │   ", DIM)),
            );
            let instruction = if agents::number(record, "received_version") == 0 {
                agents::value(record, "pending_instruction")
            } else {
                agents::value(record, "instructions")
            };
            if !instruction.is_empty() {
                rows.extend(
                    Line::new(instruction, MUTED)
                        .wrap(columns.saturating_sub(6).max(1), true)
                        .into_iter()
                        .map(|line| line.indent("  │   ", DIM)),
                );
            }
        }
        for detail in agents::list(event, "details") {
            if let Some(detail) = detail.as_str() {
                rows.extend(
                    Line::new(detail, MUTED)
                        .wrap(columns.saturating_sub(4).max(1), true)
                        .into_iter()
                        .map(|line| line.indent("  ├ ", DIM)),
                );
            }
        }
    }
    if let Some(last) = rows.last_mut() {
        // Close the tree on its final visible fact without changing the receipt.
        for span in &mut last.spans {
            if span.text == "  ├ " {
                span.text = "  └ ".into();
                break;
            }
        }
    }
    rows
}

fn wrapped(text: &str, width: usize, style: crate::tui::theme::Style, rows: &mut Vec<Line>) {
    rows.extend(Line::new(&text::clean(text), style).wrap(width.max(1), true));
}

pub(in crate::tui) fn detail_rows(record: &Value, tab: usize, width: usize) -> Vec<Line> {
    let mut rows = Vec::new();
    match tab {
        0 => {
            wrapped(
                &format!(
                    "{} · assignment v{}",
                    agents::status(record),
                    agents::number(record, "received_version")
                ),
                width,
                STRONG,
                &mut rows,
            );
            let activity = agents::value(record, "activity");
            if !activity.is_empty() {
                wrapped(&format!("Current · {activity}"), width, MUTED, &mut rows);
            }
            let updates = agents::list(record, "updates");
            if !updates.is_empty() {
                wrapped("Updates", width, STRONG, &mut rows);
                if record.get("updates_truncated") == Some(&Value::Bool(true)) {
                    wrapped("Earlier updates omitted", width, DIM, &mut rows);
                }
                for update in &updates {
                    if let Some(update) = update.as_str() {
                        wrapped(update, width, MUTED, &mut rows);
                    }
                }
            }
            if !agents::list(record, "tools").is_empty() {
                wrapped("Tools", width, STRONG, &mut rows);
            }
            for tool in agents::list(record, "tools") {
                let name = agents::value(tool, "name");
                let state = agents::value(tool, "state");
                let summary = agents::value(tool, "text");
                wrapped(
                    &format!("{name} · {state} · {summary}"),
                    width,
                    if state == "failed" { BAD } else { MUTED },
                    &mut rows,
                );
            }
            if agents::list(record, "tools").is_empty() && updates.is_empty() && activity.is_empty()
            {
                wrapped("No visible activity yet.", width, DIM, &mut rows);
            }
        }
        1 => {
            wrapped(agents::value(record, "title"), width, STRONG, &mut rows);
            wrapped(
                &format!(
                    "Current assignment · v{}",
                    agents::number(record, "received_version")
                ),
                width,
                STRONG,
                &mut rows,
            );
            wrapped(
                agents::value(record, "instructions"),
                width,
                MUTED,
                &mut rows,
            );
            let sent = agents::number(record, "sent_version");
            let received = agents::number(record, "received_version");
            if sent > received {
                wrapped(
                    &format!("Update v{sent} sent · awaiting receipt"),
                    width,
                    WARNING,
                    &mut rows,
                );
                wrapped(
                    agents::value(record, "pending_instruction"),
                    width,
                    MUTED,
                    &mut rows,
                );
            }
            wrapped(
                &format!("Workspace · {}", agents::value(record, "workspace")),
                width,
                MUTED,
                &mut rows,
            );
            wrapped(agents::value(record, "directory"), width, MUTED, &mut rows);
        }
        _ => {
            if let Some(report) = record
                .get("report")
                .filter(|report| !matches!(report, Value::Null))
            {
                wrapped(
                    &format!("Report · assignment v{}", agents::number(report, "version")),
                    width,
                    STRONG,
                    &mut rows,
                );
                wrapped(agents::value(report, "text"), width, MUTED, &mut rows);
                for file in agents::list(record, "files") {
                    if let Some(file) = file.as_str() {
                        wrapped(file, width, MUTED, &mut rows);
                    }
                }
                for check in agents::list(record, "checks") {
                    if let Some(check) = check.as_str() {
                        wrapped(
                            check,
                            width,
                            if check.to_lowercase().contains("fail") {
                                BAD
                            } else {
                                MUTED
                            },
                            &mut rows,
                        );
                    }
                }
                let reviewed = record.get("reviewed") == Some(&Value::Bool(true));
                wrapped(
                    if reviewed {
                        "Reviewed by main agent"
                    } else {
                        "Pending review by main agent"
                    },
                    width,
                    if reviewed { MUTED } else { WARNING },
                    &mut rows,
                );
            } else if agents::value(record, "state") == "unknown" {
                wrapped(
                    "No confirmed result after recovery.",
                    width,
                    WARNING,
                    &mut rows,
                );
            } else {
                wrapped("No result returned yet.", width, DIM, &mut rows);
            }
            let error = agents::value(record, "error");
            if !error.is_empty() {
                wrapped(error, width, BAD, &mut rows);
            }
        }
    }
    rows
}

fn list_rows(state: &State, panel_state: &Panel, columns: usize, room: usize) -> Vec<(Line, Part)> {
    let filtered = panel_state.filtered(&state.collaboration);
    if filtered.is_empty() {
        return vec![(
            panel::indent(
                Line::new(
                    if state.collaboration.is_empty() {
                        "No open assignments"
                    } else {
                        "No matching agents"
                    },
                    MUTED,
                ),
                columns,
            ),
            Part::Keep,
        )];
    }
    let start = panel::start(panel_state.selected, filtered.len(), room);
    let shown = filtered
        .iter()
        .enumerate()
        .skip(start)
        .take(room)
        .map(|(index, record_index)| {
            let record = &state.collaboration[*record_index];
            let on = index == panel_state.selected;
            let mut line = panel::marker(on, false);
            line = line.append(badge(
                &agents::label(record),
                agents::value(record, "state"),
            ));
            line.push(&format!("  {}", agents::value(record, "title")), STRONG);
            let status = agents::status(record);
            let right = Line::new(
                status,
                if status.starts_with("Failed") {
                    BAD
                } else if agents::value(record, "state") == "unknown" {
                    WARNING
                } else {
                    MUTED
                },
            );
            panel::choice(
                panel::spread(line, right, columns.saturating_sub(2), None),
                on,
                columns,
            )
        })
        .collect();
    panel::window(shown, start, filtered.len(), room, columns)
        .into_iter()
        .enumerate()
        .map(|(offset, line)| {
            (
                line,
                if start + offset == panel_state.selected {
                    Part::Keep
                } else {
                    Part::Row((start + offset).abs_diff(panel_state.selected))
                },
            )
        })
        .collect()
}

pub(super) fn sheet<'a>(state: &'a State, panel_state: &'a Panel, columns: usize) -> Sheet<'a> {
    let width = panel::text_width(columns);
    if !panel_state.detail {
        let filtered = panel_state.filtered(&state.collaboration);
        let room = columns.saturating_sub(3).max(1);
        let (value, caret) = panel_state.filter.viewport(room, false);
        return Sheet {
            title: panel::title(None, "Agents", "open assignments"),
            aside: Line::new(&format!("{} agents", state.collaboration.len()), MUTED),
            field: Some(Field {
                label: "Filter agents".into(),
                value,
                caret,
                placeholder: "",
                suffix: "",
            }),
            draft_caret: false,
            head: Vec::new(),
            body: Box::new(move |room| list_rows(state, panel_state, columns, room)),
            total: Some(filtered.len()),
            tail: Box::new(move |overflow| {
                panel::keys(
                    &[
                        key("↑↓", "move").scrolling(),
                        key("Enter", "inspect"),
                        key("Esc", "close"),
                    ],
                    width,
                    overflow,
                )
            }),
            confirming: false,
        };
    }
    let record = panel_state.current(&state.collaboration);
    let label = record.map(agents::label).unwrap_or_else(|| "Agent".into());
    let status = record.map(agents::status).unwrap_or("");
    let all = record
        .map(|record| detail_rows(record, panel_state.tab, width))
        .unwrap_or_default();
    let total = all.len();
    let mode = if columns >= 64 && panel_state.tab == 0 {
        Line::new(
            if panel_state.follow {
                "following"
            } else {
                "paused"
            },
            DIM,
        )
    } else {
        Line::default()
    };
    let head = panel::tabs(
        &["Activity", "Assignment", "Result"],
        panel_state.tab,
        mode,
        columns,
    )
    .to_vec();
    Sheet {
        title: panel::title(Some("Agents"), &label, "read only"),
        aside: Line::new(status, MUTED),
        field: None,
        draft_caret: false,
        head,
        body: Box::new(move |room| {
            let end = total.saturating_sub(room);
            let start = if panel_state.tab == 0 && panel_state.follow {
                end
            } else {
                panel_state.offset.min(end)
            };
            let shown = all
                .iter()
                .skip(start)
                .take(room)
                .cloned()
                .map(|line| (panel::indent(line, columns), None))
                .collect();
            panel::window(shown, start, total, room, columns)
                .into_iter()
                .enumerate()
                .map(|(index, line)| (line, Part::Row(index)))
                .collect()
        }),
        total: Some(total),
        tail: Box::new(move |overflow| {
            panel::keys(
                &[
                    key("←→", "tabs"),
                    key("↑↓", "scroll").scrolling(),
                    key("End", "follow").optional(),
                    key("Esc", "back to agents"),
                ],
                width,
                overflow,
            )
        }),
        confirming: false,
    }
}
