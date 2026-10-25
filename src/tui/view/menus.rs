use super::{
    panel::{self, Key, Panel, Part, key},
    routing,
};
use crate::tui::{
    activity,
    line::Line,
    selector::{Badge, Choice, Hit, Purpose, Selector},
    text,
    theme::{ACCENT, BAD, BODY, CURSOR, GOOD, MUTED, SELECTED_SURFACE, STRONG, Style, WARNING},
};

const LIMIT: usize = 6;

pub(super) fn selector(
    menu: &Selector,
    columns: usize,
    height: usize,
) -> (Vec<Line>, Option<(usize, usize)>) {
    let width = panel::width(columns);
    let working = matches!(menu.purpose, Purpose::Sessions { working: true, .. });
    let title = if working {
        "Deleting conversation…"
    } else {
        &menu.title
    };
    let mut out = Panel::default();
    out.add(
        panel::header(menu.parent, title, &menu.context, aside(menu), width),
        Part::Title,
    );
    out.add(panel::divider(width), Part::Header);
    out.space();
    let ending = ending(menu, width);
    match menu.purpose {
        Purpose::Key | Purpose::AutoCompact => form(menu, &mut out, width),
        Purpose::Loading => {
            let mut line = activity::indicator((menu.opened.elapsed().as_millis() / 120) as usize);
            line.push(&format!("  {}", menu.message), MUTED);
            out.extend(line.wrap(width, true), Part::Keep);
        }
        _ => list(menu, &mut out, width, height, ending.len()),
    }
    out.space();
    let part = if menu.delete_target().is_some() {
        Part::Focus
    } else {
        Part::Keep
    };
    out.extend(ending, part);
    out.finish(columns, height)
}

fn aside(menu: &Selector) -> Line {
    let count = |noun: &str| {
        let total = menu.options.len();
        if menu.filtered.len() < total {
            format!("{} of {total} {noun}", menu.filtered.len())
        } else {
            format!("{total} {noun}")
        }
    };
    let text = match &menu.purpose {
        Purpose::Models { .. } => count("models"),
        Purpose::Providers { .. } => count("providers"),
        Purpose::Accounts => format!(
            "{} accounts",
            menu.options
                .iter()
                .filter(|row| matches!(row.choice, Choice::Account(_)))
                .count()
        ),
        Purpose::Sessions { .. } if !menu.options.is_empty() => count("conversations"),
        Purpose::Efforts { defaults: true, .. } | Purpose::Settings => "new conversations".into(),
        Purpose::Routing { entries, .. } if entries.iter().any(|entry| entry.metrics.is_some()) => {
            "30-min medians · $/M after discounts".into()
        }
        Purpose::Key => "~/.jecode/config.json".into(),
        Purpose::AutoCompact => "this and new conversations".into(),
        Purpose::Drafts { .. } => {
            let queued = menu
                .options
                .iter()
                .filter(|option| option.badge == Some(Badge::Queued))
                .count();
            let paused = menu.options.len() - queued;
            [(queued, "queued"), (paused, "paused")]
                .into_iter()
                .filter(|(count, _)| *count > 0)
                .map(|(count, state)| format!("{count} {state}"))
                .collect::<Vec<_>>()
                .join(" · ")
        }
        _ => String::new(),
    };
    Line::new(&text, MUTED)
}

/// The key row, or the confirmation that replaces it.
fn ending(menu: &Selector, width: usize) -> Vec<Line> {
    let back = if menu.parent.is_some() {
        "back"
    } else {
        "close"
    };
    let close = [key("Esc", back)];
    let deleting = menu.delete_target().is_some();
    let keys: Vec<Key> = match &menu.purpose {
        Purpose::Sessions { working: true, .. } => vec![key("Ctrl+Q", "quit after cleanup")],
        Purpose::Sessions { .. } if deleting => {
            return panel::confirm(
                "Delete this conversation?",
                &[key("Enter", "delete"), key("Esc", "cancel")],
                width,
            );
        }
        Purpose::Drafts { .. } if deleting => {
            return panel::confirm(
                "Discard this draft?",
                &[key("Enter", "discard"), key("Esc", "cancel")],
                width,
            );
        }
        Purpose::Account {
            confirm_sign_out: true,
        } => {
            return panel::confirm(
                "Sign out of this account?",
                &[key("Enter", "sign out"), key("Esc", "cancel")],
                width,
            );
        }
        Purpose::Sessions { .. } | Purpose::Drafts { .. } if menu.options.is_empty() => {
            close.to_vec()
        }
        Purpose::Sessions { .. } => vec![
            key("↑↓", "move").optional(),
            key("Enter", "resume"),
            key("Ctrl+D", "delete"),
            close[0],
        ],
        Purpose::Drafts { .. } => {
            let paused = matches!(
                menu.chosen(),
                Some(Choice::Draft(crate::tui::drafts::Slot::Paused(_)))
            );
            let mut keys = vec![key("↑↓", "move").optional(), key("Enter", "edit")];
            if paused {
                keys.push(key("Ctrl+S", "send"));
            }
            keys.extend([key("Ctrl+D", "discard"), close[0]]);
            keys
        }
        Purpose::Key => vec![key("Enter", "validate and save"), close[0]],
        Purpose::AutoCompact => vec![key("0–9", "type"), key("Enter", "save"), close[0]],
        Purpose::Loading if menu.title == "Sign out" => vec![],
        Purpose::Loading => close.to_vec(),
        Purpose::Settings => vec![
            key("↑↓", "move").optional(),
            key("Enter", "change"),
            close[0],
        ],
        Purpose::Copy => vec![key("↑↓", "move").optional(), key("Enter", "copy"), close[0]],
        _ => vec![
            key("↑↓", "move").optional(),
            key("Enter", "select"),
            close[0],
        ],
    };
    panel::keys(&keys, width, true)
}

fn form(menu: &Selector, out: &mut Panel, width: usize) {
    let secret = matches!(menu.purpose, Purpose::Key);
    let (field, suffix, prose) = if secret {
        (
            width,
            "",
            "Paste a new key. Jecode validates it, then stores it in plain text; input stays hidden.",
        )
    } else {
        (
            6.min(width),
            "  % of the context window",
            "From 1 to 99. Higher values keep more history but leave less room for output.",
        )
    };
    let (value, caret) = menu.editor.viewport(field.saturating_sub(2).max(1), secret);
    let placeholder = if secret { "Paste your key" } else { "" };
    let mut line = panel::field(&value, placeholder, field, STRONG).caret(1 + caret, CURSOR);
    line.push(suffix, MUTED);
    out.caret(1 + caret);
    out.add(line, Part::Keep);
    out.add(Line::default(), Part::Strip);
    out.extend(Line::new(prose, MUTED).wrap(width, true), Part::Strip);
}

fn list(menu: &Selector, out: &mut Panel, width: usize, height: usize, ending: usize) {
    if menu.searchable {
        let (value, caret) = menu.editor.viewport(width.saturating_sub(2), false);
        let line = panel::field(&value, "Type to filter", width, STRONG).caret(1 + caret, CURSOR);
        out.caret(1 + caret);
        out.add(line, Part::Keep);
        out.space();
    }
    let empty = if menu.options.is_empty() && matches!(menu.purpose, Purpose::Sessions { .. }) {
        Some("No saved conversations")
    } else if menu.options.is_empty() && matches!(menu.purpose, Purpose::Drafts { .. }) {
        Some("No pending drafts")
    } else if menu.filtered.is_empty() {
        Some("No matching option")
    } else {
        None
    };
    if let Some(empty) = empty {
        out.add(Line::new(empty, MUTED), Part::Keep);
        return;
    }
    let table = match &menu.purpose {
        Purpose::Routing { entries, .. } => Some(entries),
        _ => None,
    };
    // Header, field and the key row always stay; rows take what is left.
    let fixed = 1 + usize::from(menu.searchable) + ending;
    let total = menu.filtered.len();
    let limit = panel::budget(height, fixed, LIMIT);
    let start = menu
        .selected
        .saturating_sub(limit / 2)
        .min(total.saturating_sub(limit));
    let overflow = total > limit;
    let rows = width.saturating_sub(2 * usize::from(overflow)).max(1);
    let columns = table.map(|entries| routing::Columns::new(entries, rows.saturating_sub(2)));
    let prose = prose(menu, columns.as_ref());
    let strip = menu.options.iter().any(|option| !option.prose.is_empty()) || !prose.is_empty();
    if let Some(columns) = columns.as_ref().filter(|columns| columns.measured()) {
        out.add(
            Line::new("  ", MUTED).append(columns.header()),
            Part::Header,
        );
    }
    let names = name_width(menu, rows);
    for (index, hit) in menu.filtered.iter().enumerate().skip(start).take(limit) {
        let on = index == menu.selected;
        let body = match (&columns, table) {
            (Some(columns), Some(entries)) => {
                let option = &menu.options[hit.index];
                let mut line = marker(on, false);
                line.spans.extend(
                    columns
                        .row(
                            &entries[hit.index],
                            option.badge.is_some(),
                            rows.saturating_sub(2),
                        )
                        .spans,
                );
                line
            }
            _ => option(menu, hit, on, names, rows),
        };
        let body = if on {
            panel::surfaced(
                panel::fit(body, rows, Some(SELECTED_SURFACE)),
                SELECTED_SURFACE,
            )
        } else {
            panel::fit(body, rows, None)
        };
        let bar = overflow.then_some((index - start, start, total, limit));
        let part = if on {
            Part::Keep
        } else {
            Part::Row(index.abs_diff(menu.selected))
        };
        out.add(panel::barred(body, bar), part);
    }
    if strip {
        // What a pending delete removes belongs to its confirmation.
        let part = if menu.delete_target().is_some() {
            Part::Keep
        } else {
            Part::Strip
        };
        out.add(Line::default(), part);
        out.extend(panel::strip(&prose, width), part);
    }
    if let Purpose::Routing { notes, .. } = &menu.purpose {
        for note in notes {
            out.add(Line::new(note, MUTED).shortened(width), Part::Strip);
        }
    }
}

/// The focused option's prose; a pending delete explains what it removes.
fn prose(menu: &Selector, columns: Option<&routing::Columns>) -> String {
    if let Purpose::Sessions { current, .. } = &menu.purpose
        && let Some(target) = menu.delete_target()
    {
        let mut prose = String::from("Deletes its history, owned outputs and temporary files.");
        if Some(target) == *current {
            prose.push_str(" A new conversation starts; unsent input is kept.");
        }
        return prose;
    }
    let Some(hit) = menu.filtered.get(menu.selected) else {
        return String::new();
    };
    let prose = &menu.options[hit.index].prose;
    let hidden = match (&menu.purpose, columns) {
        (Purpose::Routing { entries, .. }, Some(columns)) => columns.hidden(&entries[hit.index]),
        _ => String::new(),
    };
    if hidden.is_empty() {
        prose.clone()
    } else {
        format!("{hidden} · {prose}")
    }
}

/// The name column's width when options carry a fact beside the name.
fn name_width(menu: &Selector, width: usize) -> usize {
    let widest = menu
        .options
        .iter()
        .filter(|option| !option.fact.is_empty())
        .map(|option| text::cells(&text::clean(&option.name)))
        .max();
    widest.map_or(0, |widest| (widest + 2).min(width * 3 / 5))
}

pub(super) fn marker(on: bool, deleting: bool) -> Line {
    Line::new(
        if on { "› " } else { "  " },
        if deleting { BAD } else { ACCENT },
    )
}

/// `text` with the characters at `matched` in the accent.
pub(super) fn highlighted(line: &mut Line, value: &str, matched: &[usize], style: Style) {
    for (position, character) in value.chars().enumerate() {
        line.push(
            &character.to_string(),
            if matched.contains(&position) {
                ACCENT
            } else {
                style
            },
        );
    }
}

fn option(menu: &Selector, hit: &Hit, on: bool, names: usize, width: usize) -> Line {
    let value = &menu.options[hit.index];
    let deleting = menu.delete_target() == Some(hit.index);
    let mut line = marker(on, deleting);
    let style = if deleting {
        BAD
    } else if matches!(menu.purpose, Purpose::Drafts { .. }) {
        BODY
    } else {
        STRONG
    };
    let right = badge(value.badge.as_ref());
    let badge = panel::cells(&right);
    let room = width.saturating_sub(2 + badge + usize::from(badge > 0));
    let name = text::ellipsize(&text::clean(&value.name), room);
    highlighted(&mut line, &name, &hit.name, style);
    let fact = text::clean(&value.fact);
    let column = names.max(text::cells(&name) + 2);
    // A fact shows only with room for a few of its cells.
    if !fact.is_empty() && room >= column + 6 {
        line = panel::fit(line, 2 + column, None);
        highlighted(
            &mut line,
            &text::ellipsize(&fact, room - column),
            &hit.fact,
            MUTED,
        );
    }
    panel::spread(line, right, width, None)
}

pub(super) fn badge(badge: Option<&Badge>) -> Line {
    let mut line = Line::default();
    match badge {
        Some(Badge::Current) => {
            line.push("● ", GOOD);
            line.push("current", MUTED);
        }
        Some(Badge::Default) => line.push("default", MUTED),
        Some(Badge::Note(note)) => line.push(note, MUTED),
        Some(Badge::Queued) => line.push("queued", ACCENT),
        Some(Badge::Paused) => line.push("paused", WARNING),
        None => {}
    }
    line
}
