//! The commands menu: the draft is its filter. A bare slash lists every
//! command under its group; typing filters the list and drops the groups.

use super::{
    menus::highlighted,
    panel::{self, Key, Part, Sheet, key},
};
use crate::{
    session::commands::COMMANDS,
    tui::{
        line::Line,
        state::State,
        text,
        theme::{DIM, MUTED, STRONG},
    },
};

/// One list row: a group heading or a match by its position.
enum Item {
    Group(&'static str),
    Command(usize),
}

pub(super) fn sheet(state: &State, columns: usize) -> Sheet<'_> {
    let width = panel::text_width(columns);
    Sheet {
        title: panel::title(None, "Commands", ""),
        aside: Line::new(&aside(state), MUTED),
        field: None,
        draft_caret: true,
        head: Vec::new(),
        body: Box::new(move |room| body(state, columns, room)),
        total: None,
        tail: Box::new(move |_| ending(state, width)),
        confirming: false,
    }
}

fn aside(state: &State) -> String {
    let menu = &state.suggestions;
    if !menu.visible {
        String::new()
    } else if state.editor.text == "/" {
        format!("{} commands", COMMANDS.len())
    } else if menu.matches.len() == 1 {
        "1 match".into()
    } else {
        format!("{} matches", menu.matches.len())
    }
}

/// The command named by a draft that already carries arguments.
fn typed(state: &State) -> Option<&'static crate::session::commands::Command> {
    let name = state.editor.text.split_whitespace().next()?;
    COMMANDS.iter().find(|command| command.name == name)
}

fn ending(state: &State, width: usize) -> Vec<Line> {
    let menu = &state.suggestions;
    let immediate = if menu.visible {
        menu.chosen()
    } else {
        state.editor.text.split_whitespace().next()
    }
    .is_some_and(crate::tui::commands::immediate);
    let action = if state.activity.is_some() && !immediate {
        "queue"
    } else {
        "run"
    };
    let mut keys: Vec<Key> = Vec::new();
    if menu.visible && !menu.matches.is_empty() {
        keys.extend([
            key("↑↓", "move").optional(),
            key("Tab", "complete").optional(),
        ]);
    }
    keys.extend([key("Enter", action), key("Esc", "close")]);
    panel::keys(&keys, width, true)
}

fn body(state: &State, columns: usize, room: usize) -> Vec<(Line, Part)> {
    let menu = &state.suggestions;
    if !menu.visible {
        // A command with arguments keeps its description below the composer.
        return typed(state)
            .map(|command| {
                let line = Line::new(&text::clean(command.description(true)), MUTED);
                (panel::indent(line, columns), Part::Keep)
            })
            .into_iter()
            .collect();
    }
    if menu.matches.is_empty() {
        let line = Line::new("No matching command", MUTED);
        return vec![(panel::indent(line, columns), Part::Keep)];
    }
    let mut items = Vec::new();
    let mut selected = 0;
    for (position, matched) in menu.matches.iter().enumerate() {
        let group = COMMANDS[matched.index].group();
        let previous = position
            .checked_sub(1)
            .map(|before| COMMANDS[menu.matches[before].index].group());
        if state.editor.text == "/" && previous != Some(group) {
            items.push(Item::Group(group));
        }
        if position == menu.selected {
            selected = items.len();
        }
        items.push(Item::Command(position));
    }
    let total = items.len();
    let start = panel::start(selected, total, room);
    let names = COMMANDS
        .iter()
        .map(|command| text::cells(command.tui_usage))
        .max()
        .unwrap_or(0)
        + 2;
    let names = names.min(panel::text_width(columns) * 3 / 5);
    let shown = items
        .iter()
        .enumerate()
        .skip(start)
        .take(room)
        .map(|(index, item)| {
            let on = index == selected;
            let content = match item {
                Item::Group(group) => Line::new(&format!("  {group}"), DIM),
                Item::Command(position) => row(state, *position, on, names, columns),
            };
            panel::choice(content, on, columns)
        })
        .collect();
    panel::window(shown, start, total, room, columns)
        .into_iter()
        .enumerate()
        .map(|(offset, line)| {
            let index = start + offset;
            let part = if index == selected {
                Part::Keep
            } else {
                Part::Row(index.abs_diff(selected))
            };
            (line, part)
        })
        .collect()
}

/// The command with its arguments, its description in the fact column and
/// the one shortcut a command has.
fn row(state: &State, position: usize, on: bool, names: usize, columns: usize) -> Line {
    let matched = &state.suggestions.matches[position];
    let command = &COMMANDS[matched.index];
    let end = columns.saturating_sub(2);
    let mut line = panel::marker(on, false);
    highlighted(&mut line, command.name, &matched.positions, STRONG);
    let arguments = command.tui_usage.strip_prefix(command.name).unwrap_or("");
    line.push(arguments, MUTED);
    let shortcut = if command.name == "/drafts" {
        Line::new("Alt+↑", MUTED)
    } else {
        Line::default()
    };
    let room =
        end.saturating_sub(panel::cells(&shortcut) + 2 * usize::from(command.name == "/drafts"));
    if room >= 2 + names + 6 {
        line = panel::fit(line, 2 + names, None);
        line.push(
            &text::ellipsize(command.description(true), room - 2 - names),
            MUTED,
        );
    }
    panel::spread(line, shortcut, end, None)
}
