use super::panel;
use crate::{
    session::commands::COMMANDS,
    tui::{
        line::Line,
        selector::{Purpose, Selector},
        suggestions::Suggestions,
        text,
        theme::{ACCENT, BAD, CURSOR, MUTED, PROMPT, SELECTED_BACKGROUND},
    },
};

pub(super) fn suggestions(menu: &Suggestions, columns: usize, limit: usize) -> (Vec<Line>, String) {
    if menu.matches.is_empty() {
        return (vec![Line::new("No matching command", MUTED)], String::new());
    }
    let limit = limit.clamp(1, 6);
    let start = menu.selected.saturating_sub(limit - 1);
    let lines = menu
        .matches
        .iter()
        .enumerate()
        .skip(start)
        .take(limit)
        .map(|(index, matched)| {
            let command = &COMMANDS[matched.index];
            option(
                command.name,
                command.description,
                &matched.positions,
                &[],
                index == menu.selected,
                ("", false),
                columns,
            )
        })
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
    if menu.searchable || secret {
        let prefix = if width < 7 { "" } else { "› " };
        let (value, caret) = menu
            .editor
            .viewport(width.saturating_sub(text::cells(prefix)), secret);
        let mut line = Line::new(prefix, ACCENT);
        line.push(&value, PROMPT);
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
    let available = height.saturating_sub(lines.len() + 3 + details);
    let limit = available.clamp(1, 8);
    let start = menu.selected.saturating_sub(limit - 1);
    let mut selected = None;
    for (position, hit) in menu.filtered.iter().enumerate().skip(start).take(limit) {
        if position == menu.selected {
            selected = Some(lines.len());
        }
        let value = &menu.options[hit.index];
        let number = if deleting == Some(hit.index) {
            if working {
                String::new()
            } else {
                if drafts {
                    "Discard? ".into()
                } else {
                    "Delete? ".into()
                }
            }
        } else if menu.searchable {
            String::new()
        } else {
            format!("{}. ", position + 1)
        };
        lines.push(option(
            &value.name,
            &value.description,
            &hit.name,
            &hit.description,
            position == menu.selected,
            (&number, deleting == Some(hit.index)),
            width,
        ));
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
    if details > 0 {
        lines.push(Line::new(
            "History, owned outputs and temporary files",
            MUTED,
        ));
        if current {
            lines.push(Line::new("New conversation · unsent input kept", MUTED));
        }
    }
    let controls: &[&str] = if menu.options.is_empty()
        && matches!(
            menu.purpose,
            Purpose::Sessions { .. } | Purpose::Drafts { .. }
        ) {
        &["Esc close", "Esc"]
    } else if working {
        &["Ctrl+Q quit after cleanup", "Ctrl+Q quit", "Ctrl+Q"]
    } else if deleting.is_some() && drafts {
        &[
            "Enter discard · Esc cancel · Ctrl+D cancel",
            "Enter discard · Esc cancel",
            "↵ discard · Esc",
        ]
    } else if deleting.is_some() {
        &[
            "Enter delete · Esc cancel · Ctrl+D cancel",
            "Enter delete · Esc cancel",
            "Enter delete",
        ]
    } else if drafts {
        let paused = menu.chosen().is_some_and(|choice| match choice {
            crate::tui::selector::Choice::Draft(index) => {
                menu.options[index].description == "Paused"
            }
            _ => false,
        });
        if paused {
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
    } else if matches!(menu.purpose, Purpose::Sessions { .. }) {
        if menu.searchable {
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
    } else if matches!(menu.purpose, Purpose::Key) {
        &[
            "Enter validate and save · Esc close · key stored in plain text",
            "Enter save · Esc close · plain text key",
            "Enter save · Esc",
            "↵ · Esc",
        ]
    } else if matches!(menu.purpose, Purpose::Loading) {
        &["Esc close", "Esc"]
    } else if menu.searchable {
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
    };
    lines.push(Line::new(
        panel::hint(controls, width),
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

fn option(
    name: &str,
    description: &str,
    matched: &[usize],
    description_matches: &[usize],
    selected: bool,
    number: (&str, bool),
    columns: usize,
) -> Line {
    let (number, deleting) = number;
    let mut line = Line::new(
        if selected { "› " } else { "  " },
        if deleting {
            BAD
        } else if selected {
            ACCENT
        } else {
            MUTED
        },
    );
    line.push(number, if deleting { BAD } else { MUTED });
    let name = text::ellipsize(&text::clean(name), columns.saturating_sub(5));
    for (position, character) in name.chars().enumerate() {
        line.push(
            &character.to_string(),
            if deleting {
                BAD
            } else if matched.contains(&position) {
                ACCENT
            } else {
                PROMPT
            },
        );
    }
    let used = text::cells(&line.plain());
    if columns > used + 4 {
        let gap = (16usize.saturating_sub(used))
            .max(2)
            .min(columns - used - 1);
        line.push(&" ".repeat(gap), MUTED);
        let description = text::ellipsize(&text::clean(description), columns - used - gap);
        for (index, character) in description.chars().enumerate() {
            line.push(
                &character.to_string(),
                if description_matches.contains(&index) {
                    ACCENT
                } else {
                    MUTED
                },
            );
        }
    }
    if selected {
        line.background = Some(SELECTED_BACKGROUND);
    }
    line
}
