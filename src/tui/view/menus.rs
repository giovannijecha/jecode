use super::panel;
use crate::{
    session::commands::COMMANDS,
    tui::{
        hint,
        line::Line,
        selector::{Purpose, Selector},
        suggestions::Suggestions,
        text,
        theme::{ACCENT, BAD, CURSOR, MUTED, SELECTED_SURFACE, STRONG, Style},
    },
};

pub(super) fn suggestions(menu: &Suggestions, columns: usize, limit: usize) -> (Vec<Line>, String) {
    if menu.matches.is_empty() {
        return (vec![Line::new("No matching command", MUTED)], String::new());
    }
    let limit = limit.clamp(1, 6);
    let start = menu.selected.saturating_sub(limit - 1);
    let rows: Vec<_> = menu
        .matches
        .iter()
        .enumerate()
        .map(|(index, matched)| {
            let command = &COMMANDS[matched.index];
            Row {
                name: command.name,
                description: command.description,
                matched: &matched.positions,
                description_matches: &[],
                description_style: MUTED,
                number: String::new(),
                selected: index == menu.selected,
                deleting: false,
            }
        })
        .collect();
    let lines = rows
        .iter()
        .skip(start)
        .take(limit)
        .map(|row| row.line(columns))
        .collect();
    (
        lines,
        panel::position(
            start,
            (start + limit).min(menu.matches.len()),
            menu.matches.len(),
        ),
    )
}

pub(super) fn selector(
    menu: &Selector,
    columns: usize,
    height: usize,
) -> (Vec<Line>, Option<(usize, usize)>) {
    let width = panel::width(columns);
    let working = matches!(menu.purpose, Purpose::Sessions { working: true, .. });
    let heading = if working {
        "Deleting conversation…"
    } else {
        &menu.title
    };
    let secret = matches!(menu.purpose, Purpose::Key);
    let mut lines = vec![panel::heading(heading, "", width)];
    let mut cursor = None;
    if menu.accepts_text() {
        let prefix = if width < 7 { "" } else { "› " };
        let (value, caret) = menu
            .editor
            .viewport(width.saturating_sub(text::cells(prefix)), secret);
        let mut line = Line::new(prefix, ACCENT);
        line.push(&value, STRONG);
        let column = text::cells(prefix) + caret;
        line = line.caret(column, CURSOR);
        cursor = Some((lines.len(), column));
        lines.push(line);
    }
    let deleting = menu.delete_target();
    let current = matches!(&menu.purpose, Purpose::Sessions { current, .. }
        if deleting.is_some() && deleting == *current);
    let drafts = matches!(menu.purpose, Purpose::Drafts { .. });
    let details = if deleting.is_some() && !working && !drafts {
        1 + usize::from(current)
    } else {
        0
    };
    // Padding and controls follow the options; help rows yield to three options.
    let reserved = lines.len() + 3 + details;
    let mut help = routing_help(menu, width);
    help.truncate(height.saturating_sub(reserved + menu.filtered.len().clamp(1, 3)));
    let available = height.saturating_sub(reserved + help.len());
    let limit = available.clamp(1, 8);
    let start = menu.selected.saturating_sub(limit - 1);
    let rows: Vec<_> = menu
        .filtered
        .iter()
        .enumerate()
        .map(|(position, hit)| {
            let value = &menu.options[hit.index];
            Row {
                name: &value.name,
                description: &value.description,
                matched: &hit.name,
                description_matches: &hit.description,
                description_style: MUTED,
                number: if deleting == Some(hit.index) && !working {
                    if drafts { "Discard? " } else { "Delete? " }.into()
                } else if menu.searchable {
                    String::new()
                } else {
                    format!("{}. ", position + 1)
                },
                selected: position == menu.selected,
                deleting: deleting == Some(hit.index),
            }
        })
        .collect();
    let mut selected = None;
    for row in rows.iter().skip(start).take(limit) {
        if row.selected {
            selected = Some(lines.len());
        }
        lines.push(row.line(width));
    }
    if menu.options.is_empty() && matches!(menu.purpose, Purpose::Sessions { .. }) {
        selected = Some(lines.len());
        lines.push(Line::new("No saved conversations", MUTED));
    } else if menu.options.is_empty() && drafts {
        selected = Some(lines.len());
        lines.push(Line::new("No pending drafts", MUTED));
    } else if menu.filtered.is_empty() && menu.searchable {
        selected = Some(lines.len());
        lines.push(Line::new("No matching option", MUTED));
    }
    let mut position = panel::position(
        start,
        (start + limit).min(menu.filtered.len()),
        menu.filtered.len(),
    );
    if position.is_empty() && menu.filtered.len() < menu.options.len() && !menu.filtered.is_empty()
    {
        position = format!(
            "{} {}",
            menu.filtered.len(),
            if menu.filtered.len() == 1 {
                "match"
            } else {
                "matches"
            }
        );
    }
    lines[0] = panel::heading(heading, &position, width);
    if matches!(menu.purpose, Purpose::AutoCompact) {
        lines.push(Line::new(
            &hint::choose(
                &[
                    "1–99% of the context window · higher values leave less room for output",
                    "1–99% of the context window · less output room when higher",
                    "1–99% of the context window",
                    "1–99%",
                ],
                width,
            ),
            MUTED,
        ));
    }
    lines.extend(help);
    if details > 0 {
        lines.push(Line::new(
            "History, owned outputs and temporary files",
            MUTED,
        ));
        if current {
            lines.push(Line::new("New conversation · unsent input kept", MUTED));
        }
    }
    lines.push(Line::new(
        &source_controls(menu, deleting.is_some(), working, width),
        if deleting.is_some() { BAD } else { MUTED },
    ));
    let mut priorities = Vec::new();
    if working {
        priorities.push(0);
    }
    if deleting.is_some() {
        priorities.extend(selected);
        priorities.push(lines.len() - 1);
    }
    priorities.extend(cursor.map(|(row, _)| row));
    priorities.extend(selected);
    if cursor.is_none() && selected.is_none() {
        priorities.push(0);
    }
    priorities.extend([lines.len() - 1, 0]);
    panel::finish(lines, cursor, &priorities, columns, height)
}

/// What the focused routing does, in up to two rows, then one row per note.
fn routing_help(menu: &Selector, width: usize) -> Vec<Line> {
    let Purpose::Routing { notes, .. } = &menu.purpose else {
        return vec![];
    };
    let help = menu.routing_help().unwrap_or_default();
    let mut rows = Line::new(help, MUTED).wrap(width, true);
    // Two rows at most; a cut second row ends with an ellipsis.
    if rows.len() > 2 {
        let first = rows[0].plain();
        let rest = help
            .strip_prefix(first.trim_end())
            .unwrap_or(help)
            .trim_start();
        rows.truncate(1);
        rows.push(Line::new(&text::ellipsize(rest, width), MUTED));
    }
    rows.extend(notes.iter().map(|note| Line::new(note, MUTED)));
    rows
}

fn source_controls(menu: &Selector, deleting: bool, working: bool, width: usize) -> String {
    let variants: &[&str] = if matches!(menu.purpose, Purpose::Sessions { .. }) {
        if menu.options.is_empty() {
            &["Esc close", "Esc"]
        } else if working {
            &["Ctrl+Q quit after cleanup", "Ctrl+Q quit", "Ctrl+Q"]
        } else if deleting {
            &[
                "Enter delete · Esc cancel · Ctrl+D cancel",
                "Enter delete · Esc cancel",
                "Enter delete",
            ]
        } else if menu.searchable {
            &[
                "↑↓ move · Enter resume · Ctrl+D delete · Esc close",
                "↑↓ · Enter · Ctrl+D delete · Esc",
                "↵ · ^D · Esc",
            ]
        } else {
            &[
                "↑↓ move · Enter resume · Ctrl+D delete · 1–8 choose · Esc close",
                "↑↓ · Enter · Ctrl+D delete · Esc",
                "↵ · ^D · Esc",
            ]
        }
    } else if matches!(menu.purpose, Purpose::Drafts { .. }) {
        if menu.options.is_empty() {
            &["Esc close", "Esc"]
        } else if deleting {
            &[
                "Enter discard · Esc cancel · Ctrl+D cancel",
                "Enter discard · Esc cancel",
                "↵ discard · Esc",
            ]
        } else if matches!(
            menu.chosen(),
            Some(crate::tui::selector::Choice::Draft(
                crate::tui::drafts::Slot::Paused(_)
            ))
        ) {
            &[
                "↑↓ move · Enter edit · Ctrl+S send · Ctrl+D discard · Esc close",
                "↑↓ · Enter edit · ^S send · ^D discard · Esc",
                "↵ edit · ^S send · ^D discard · Esc",
                "↵ · ^S · ^D · Esc",
            ]
        } else {
            &[
                "↑↓ move · Enter edit · Ctrl+D discard · Esc close",
                "↑↓ · Enter edit · ^D discard · Esc",
                "↵ edit · ^D discard · Esc",
                "↵ · ^D · Esc",
            ]
        }
    } else if matches!(menu.purpose, Purpose::Key) {
        &[
            "Enter validate and save · Esc close · key stored in plain text",
            "Enter save · Esc · key stored in plain text",
            "↵ · Esc · key stored in plain text",
            "Enter save · Esc",
            "↵ · Esc",
        ]
    } else if matches!(menu.purpose, Purpose::AutoCompact) {
        &[
            "Enter save · Esc close · applies to this and new conversations",
            "Enter save · Esc · applies to this and new conversations",
            "Enter save · Esc close",
            "↵ · Esc",
        ]
    } else if matches!(menu.purpose, Purpose::Loading) {
        &["Esc close", "Esc"]
    } else {
        if menu.searchable {
            &[
                "↑↓ move · Enter select · Esc close",
                "↑↓ · Enter · Esc",
                "↑↓ · ↵ · Esc",
            ]
        } else {
            &[
                "↑↓ move · Enter select · 1–8 choose · Esc close",
                "↑↓ · Enter · 1–8 · Esc",
                "↑↓ · ↵ · Esc",
            ]
        }
    };
    hint::choose(variants, width)
}

/// One option of a command or selector list.
pub(super) struct Row<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub matched: &'a [usize],
    pub description_matches: &'a [usize],
    pub description_style: Style,
    pub number: String,
    pub selected: bool,
    pub deleting: bool,
}

const DESCRIPTION: usize = 16;

impl Row<'_> {
    pub(super) fn line(&self, columns: usize) -> Line {
        // A marked name turns red; its description keeps the source role.
        let tone = |style| if self.deleting { BAD } else { style };
        let mut line = Line::new(
            if self.selected { "› " } else { "  " },
            tone(if self.selected { ACCENT } else { MUTED }),
        );
        line.push(&self.number, tone(MUTED));
        let room = columns.saturating_sub(5);
        let name = text::ellipsize(&text::clean(self.name), room);
        for (position, character) in name.chars().enumerate() {
            line.push(
                &character.to_string(),
                tone(if self.matched.contains(&position) {
                    ACCENT
                } else {
                    STRONG
                }),
            );
        }
        let used = text::cells(&line.plain());
        if columns > used + 4 {
            let gap = DESCRIPTION
                .saturating_sub(used)
                .max(2)
                .min(columns - used - 1);
            line.push(&" ".repeat(gap), MUTED);
            let description = text::ellipsize(&text::clean(self.description), columns - used - gap);
            for (index, character) in description.chars().enumerate() {
                line.push(
                    &character.to_string(),
                    if self.description_matches.contains(&index) {
                        ACCENT
                    } else {
                        self.description_style
                    },
                );
            }
        }
        if self.selected {
            line.background = Some(SELECTED_SURFACE);
        }
        line
    }
}
