//! The commands menu: the draft is its search field. A bare slash lists every
//! command under its group; typing filters the list and drops the groups.

use super::{
    composer,
    menus::{highlighted, marker},
    panel::{self, Key, Panel, Part, key},
};
use crate::{
    session::commands::COMMANDS,
    tui::{
        line::Line,
        state::State,
        text,
        theme::{ACCENT, CODE_SURFACE, MUTED, SELECTED_SURFACE, STRONG, TOOL_DIM},
    },
};

const LIMIT: usize = 6;

/// One list row: a group heading or a match by its position.
enum Item {
    Group(&'static str),
    Command(usize),
}

pub(super) fn render(
    state: &State,
    columns: usize,
    height: usize,
) -> (Vec<Line>, Option<(usize, usize)>) {
    let width = panel::width(columns);
    let menu = &state.suggestions;
    let mut out = Panel::default();
    out.add(
        panel::header(None, "Commands", "", Line::new(&aside(state), MUTED), width),
        Part::Title,
    );
    out.add(panel::divider(width), Part::Header);
    out.space();
    let ending = ending(state, width);
    let fixed = 1 + ending.len();
    let input = composer::input(
        state,
        width,
        ACCENT,
        STRONG,
        height.saturating_sub(fixed + 1),
    );
    for (row, line) in input.lines.into_iter().enumerate() {
        // Below seven columns the composer omits its marker and uses the
        // entire row for the editor, including a caret at column zero.
        let (field, caret) = if width < 7 {
            (line, input.cursor.1)
        } else {
            let mut field = Line::new(" ", MUTED);
            field.spans.extend(line.spans.into_iter().skip(1));
            (field, input.cursor.1 - 1)
        };
        if row == input.cursor.0 {
            out.caret(caret);
        }
        out.add(
            panel::surfaced(panel::fit(field, width, Some(CODE_SURFACE)), CODE_SURFACE),
            Part::Keep,
        );
    }
    let fixed = fixed + out.len().saturating_sub(3);
    if menu.visible {
        out.space();
        list(state, &mut out, width, height, fixed);
    } else if let Some(command) = typed(state) {
        out.add(Line::default(), Part::Strip);
        out.extend(panel::strip(command.description(true), width), Part::Strip);
    }
    out.space();
    out.extend(ending, Part::Keep);
    out.finish(columns, height)
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

fn list(state: &State, out: &mut Panel, width: usize, height: usize, fixed: usize) {
    let menu = &state.suggestions;
    if menu.matches.is_empty() {
        out.add(Line::new("No matching command", MUTED), Part::Keep);
        return;
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
    let limit = panel::budget(height, fixed, LIMIT);
    let start = selected
        .saturating_sub(limit / 2)
        .min(total.saturating_sub(limit));
    let overflow = total > limit;
    let rows = width.saturating_sub(2 * usize::from(overflow)).max(1);
    let names = COMMANDS
        .iter()
        .map(|command| text::cells(command.tui_usage))
        .max()
        .unwrap_or(0)
        + 2;
    for (index, item) in items.iter().enumerate().skip(start).take(limit) {
        let on = index == selected;
        let line = match item {
            Item::Group(group) => panel::fit(Line::new(group, TOOL_DIM), rows, None),
            Item::Command(position) => row(state, *position, on, names.min(rows), rows),
        };
        let part = if on {
            Part::Keep
        } else {
            Part::Row(index.abs_diff(selected))
        };
        let bar = overflow.then_some((index - start, start, total, limit));
        out.add(panel::barred(line, bar), part);
    }
    let description = COMMANDS[menu.matches[menu.selected].index].description(true);
    out.add(Line::default(), Part::Strip);
    out.extend(panel::strip(description, width), Part::Strip);
}

fn row(state: &State, position: usize, on: bool, names: usize, width: usize) -> Line {
    let matched = &state.suggestions.matches[position];
    let command = &COMMANDS[matched.index];
    let mut line = marker(on, false);
    highlighted(&mut line, command.name, &matched.positions, STRONG);
    let arguments = command.tui_usage.strip_prefix(command.name).unwrap_or("");
    line.push(arguments, MUTED);
    let line = panel::fit(line, names + 2, None);
    // The one command with its own shortcut shows it.
    let shortcut = if command.name == "/drafts" {
        Line::new("Alt+↑", MUTED)
    } else {
        Line::default()
    };
    let line = panel::spread(line, shortcut, width, None);
    if on {
        panel::surfaced(
            panel::fit(line, width, Some(SELECTED_SURFACE)),
            SELECTED_SURFACE,
        )
    } else {
        line
    }
}
