use super::{
    commands,
    panel::{self, Field, Key, Part, Sheet, key},
    routing,
};
use crate::tui::{
    line::Line,
    lsp::{Field as LspField, Panel as LspPanel},
    mcp::{Field as McpField, Panel as McpPanel},
    motion,
    selector::{Badge, Choice, Hit, Purpose, Selector},
    text,
    theme::{ACCENT, BAD, BODY, LIT, MUTED, STRONG, Style},
};

/// The word `current` and the gap before it.
const CHIP: usize = 9;

pub(super) fn sheet(menu: &Selector, columns: usize) -> Sheet<'_> {
    let width = panel::text_width(columns);
    let working = matches!(menu.purpose, Purpose::Sessions { working: true, .. });
    let title = if working {
        "Deleting conversation…"
    } else {
        &menu.title
    };
    let head = match &menu.purpose {
        // The state leads in its own ink; the endpoint follows.
        Purpose::Mcp(McpPanel::Detail(health)) => menu
            .message
            .lines()
            .enumerate()
            .map(|(index, value)| {
                let style = if index == 0 { health.style() } else { MUTED };
                panel::indent(Line::new(value, style), columns)
            })
            .collect(),
        Purpose::Lsp(LspPanel::Detail | LspPanel::Form | LspPanel::Instance) => menu
            .message
            .lines()
            .flat_map(|value| {
                let value = if columns <= 44 && value.starts_with("Runs as") {
                    "Runs as · you · not limited by levels"
                } else {
                    value
                };
                Line::new(value, MUTED)
                    .wrap(width, true)
                    .into_iter()
                    .map(|row| panel::indent(row, columns))
                    .collect::<Vec<_>>()
            })
            .collect(),
        Purpose::Routing { entries, .. } if !menu.filtered.is_empty() => {
            let columns = routing::Columns::new(entries, table_width(menu, columns));
            columns
                .measured()
                .then(|| Line::new("  ", MUTED).append(columns.header()))
                .into_iter()
                .collect()
        }
        _ => Vec::new(),
    };
    let body: panel::Body = match menu.purpose {
        Purpose::Key(_)
        | Purpose::AutoCompact
        | Purpose::Mcp(McpPanel::Field(_))
        | Purpose::Lsp(LspPanel::Field(_)) => Box::new(|_| Vec::new()),
        // The message wraps so a sign-in link stays whole.
        Purpose::Loading | Purpose::Mcp(McpPanel::Loading) | Purpose::Lsp(LspPanel::Checking) => {
            Box::new(move |_| {
                let frame = motion::frame();
                let mut line = motion::caret(frame, None);
                line.push(" ", MUTED);
                line = line.append(motion::shimmer(&menu.message, frame, false));
                line.wrap(width, true)
                    .into_iter()
                    .map(|row| (panel::indent(row, columns), Part::Keep))
                    .collect()
            })
        }
        Purpose::Mcp(McpPanel::List | McpPanel::Methods) if menu.options.is_empty() => {
            Box::new(move |_| {
                menu.message
                    .split('·')
                    .map(|piece| {
                        (
                            panel::indent(Line::new(piece.trim(), MUTED), columns),
                            Part::Keep,
                        )
                    })
                    .collect()
            })
        }
        Purpose::Lsp(LspPanel::List) if menu.options.is_empty() => Box::new(move |_| {
            menu.message
                .split('·')
                .map(|piece| {
                    (
                        panel::indent(Line::new(piece.trim(), MUTED), columns),
                        Part::Keep,
                    )
                })
                .collect()
        }),
        Purpose::Commands { .. } => Box::new(move |room| commands::palette(menu, columns, room)),
        _ => Box::new(move |room| list(menu, columns, room)),
    };
    Sheet {
        title: panel::title(menu.parent, title, &menu.context),
        aside: aside(menu),
        field: field(menu),
        draft_caret: false,
        head,
        body,
        total: None,
        tail: Box::new(move |_| ending(menu, width)),
        confirming: menu.delete_target().is_some()
            || matches!(
                menu.purpose,
                Purpose::Account {
                    confirm_sign_out: true
                }
            ),
    }
}

/// The value typed for this menu. The label names the field and the title the panel, so
/// a placeholder only adds what neither says.
fn field(menu: &Selector) -> Option<Field> {
    let (label, placeholder, suffix) = match &menu.purpose {
        Purpose::Key(_) => ("Paste your key".to_string(), "", ""),
        Purpose::AutoCompact => ("Threshold".into(), "1–99", "% of the context window"),
        Purpose::Mcp(McpPanel::Field(field)) => match field {
            McpField::Name => ("Shown in /mcp and tool cards".into(), "", ""),
            McpField::Endpoint => ("Server address".into(), "https://…", ""),
            McpField::Token => ("Paste your token".into(), "", ""),
            McpField::Timeout => ("Per call".into(), "1–600", " seconds"),
            McpField::Header => ("Name: value · Name: removes".into(), "Name: value", ""),
            McpField::ClientId => ("Public OAuth client ID".into(), "", ""),
        },
        Purpose::Lsp(LspPanel::Field(field)) => match field {
            LspField::Name => ("Shown in /lsp and tool cards".into(), "", ""),
            LspField::Executable => ("User-selected executable or absolute path".into(), "", ""),
            LspField::Languages => (
                "Language ID and file extensions".into(),
                "rust: .rs; cpp: .cc .cpp",
                "",
            ),
            LspField::Arguments => (
                "Quoted arguments keep spaces".into(),
                "--flag \"two words\"",
                "",
            ),
            LspField::Environment => (
                "NAME=value · NAME= removes · values hidden".into(),
                "NAME=value",
                "",
            ),
            LspField::RootMarkers => (
                "Comma-separated files inside the workspace".into(),
                "Cargo.toml",
                "",
            ),
            LspField::RootOverride => ("Explicit analysis root".into(), "Automatic", ""),
            LspField::Initialization | LspField::Settings => ("JSON value".into(), "{}", ""),
            LspField::Timeout => ("Per request".into(), "100–120000", " ms"),
        },
        Purpose::Commands { .. } => ("Filter".into(), "", ""),
        _ if menu.searchable => (format!("Filter {}", noun(menu)), "", ""),
        _ => return None,
    };
    // The whole value: the sheet wraps it and keeps the caret in view.
    let (value, caret) = menu.editor.viewport(
        usize::MAX,
        matches!(
            menu.purpose,
            Purpose::Key(_)
                | Purpose::Mcp(McpPanel::Field(McpField::Token | McpField::Header))
                | Purpose::Lsp(LspPanel::Field(LspField::Environment))
        ),
    );
    Some(Field {
        label,
        value,
        caret,
        placeholder,
        suffix,
    })
}

fn noun(menu: &Selector) -> String {
    match &menu.purpose {
        Purpose::Models { .. } => "models".into(),
        Purpose::Providers { .. } => "providers".into(),
        Purpose::ProviderGroups { .. } => "provider groups".into(),
        Purpose::Sessions { .. } => "conversations".into(),
        _ => menu.title.to_lowercase(),
    }
}

fn aside(menu: &Selector) -> Line {
    let count = |noun: &str| {
        let total = menu.options.len();
        let all = text::counted(total, noun);
        if menu.filtered.len() < total {
            format!("{} of {all}", menu.filtered.len())
        } else {
            all
        }
    };
    let text = match &menu.purpose {
        Purpose::Commands { .. } => count("command"),
        Purpose::Models { .. } => count("model"),
        Purpose::Providers { .. } => count("provider"),
        Purpose::ProviderGroups { .. } => count("group"),
        Purpose::Accounts => text::counted(
            menu.options
                .iter()
                .filter(|row| matches!(row.choice, Choice::Account(_)))
                .count(),
            "account",
        ),
        Purpose::Sessions { .. } if !menu.options.is_empty() => count("conversation"),
        Purpose::Efforts { defaults: true, .. } | Purpose::Settings => "new conversations".into(),
        Purpose::Routing { entries, .. } if entries.iter().any(|entry| entry.metrics.is_some()) => {
            "30-min medians · $/M after discounts".into()
        }
        Purpose::Permissions { default } => format!("new conversations use {}", default.name()),
        Purpose::DefaultPermissions => "new conversations".into(),
        Purpose::Remembered {
            paused: Some(level),
            ..
        } => format!("paused at {}", level.name()),
        Purpose::Remembered { .. } => "this conversation".into(),
        Purpose::Key(_) => "~/.jecode/config.json".into(),
        Purpose::AutoCompact | Purpose::Context => "this and new conversations".into(),
        Purpose::PromptCache => "Claude models".into(),
        Purpose::Theme => "this and new conversations".into(),
        Purpose::Mcp(McpPanel::List) => menu.message.clone(),
        Purpose::Mcp(McpPanel::Tools) => count("tool"),
        Purpose::Lsp(LspPanel::List) => menu.message.clone(),
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
                "Delete this conversation and its files?",
                &[key("Enter", "delete"), key("Esc", "cancel")],
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
        Purpose::Sessions { .. } | Purpose::Remembered { .. } if menu.options.is_empty() => {
            close.to_vec()
        }
        Purpose::Sessions { .. } => vec![
            key("↑↓", "move").optional(),
            key("Enter", "resume"),
            key("Ctrl+D", "delete"),
            close[0],
        ],
        Purpose::Remembered { .. } => vec![
            key("↑↓", "move").optional(),
            key("Ctrl+D", "revoke"),
            close[0],
        ],
        Purpose::Key(_) => vec![key("Enter", "validate and save"), close[0]],
        Purpose::AutoCompact => vec![key("Enter", "save"), close[0]],
        Purpose::Mcp(McpPanel::Field(McpField::Endpoint)) => vec![key("Enter", "check"), close[0]],
        Purpose::Mcp(McpPanel::Field(_)) => vec![key("Enter", "done"), close[0]],
        Purpose::Mcp(McpPanel::ConfirmRemove) => {
            return panel::confirm(
                "Remove this connection and saved sign-in?",
                &[key("Enter", "remove"), key("Esc", "cancel")],
                width,
            );
        }
        Purpose::Mcp(McpPanel::List) if menu.options.is_empty() => {
            vec![key("Enter", "open settings"), close[0]]
        }
        Purpose::Mcp(McpPanel::Loading) => close.to_vec(),
        Purpose::Lsp(LspPanel::Field(_)) => vec![key("Enter", "done"), close[0]],
        Purpose::Lsp(LspPanel::ConfirmRemove) => {
            return panel::confirm(
                "Remove this language server?",
                &[key("Enter", "remove"), key("Esc", "cancel")],
                width,
            );
        }
        Purpose::Lsp(LspPanel::List) if menu.options.is_empty() => {
            vec![key("Enter", "open settings"), close[0]]
        }
        Purpose::Lsp(LspPanel::Checking) => close.to_vec(),
        Purpose::Loading if menu.title == "Sign out" => vec![],
        Purpose::Loading => close.to_vec(),
        Purpose::Settings | Purpose::Context => vec![
            key("↑↓", "move").optional(),
            key("Enter", "change"),
            close[0],
        ],
        Purpose::Commands { .. } if menu.chosen().is_some() => {
            vec![key("↑↓", "move").optional(), key("Enter", "run"), close[0]]
        }
        Purpose::Commands { .. } => close.to_vec(),
        Purpose::Copy => vec![key("↑↓", "move").optional(), key("Enter", "copy"), close[0]],
        _ => vec![
            key("↑↓", "move").optional(),
            key("Enter", "choose"),
            close[0],
        ],
    };
    panel::keys(&keys, width, true)
}

/// The routing table's width beside the marker and the word `current`.
fn table_width(menu: &Selector, columns: usize) -> usize {
    let chip = if menu.options.iter().any(|option| option.badge.is_some()) {
        CHIP
    } else {
        0
    };
    columns.saturating_sub(4 + chip).max(1)
}

fn list(menu: &Selector, columns: usize, room: usize) -> Vec<(Line, Part)> {
    let empty = if menu.options.is_empty() && matches!(menu.purpose, Purpose::Sessions { .. }) {
        Some("No saved conversations")
    } else if menu.options.is_empty() && matches!(menu.purpose, Purpose::Remembered { .. }) {
        Some("Nothing remembered")
    } else if menu.filtered.is_empty() {
        Some("No matching option")
    } else {
        None
    };
    let notes = match &menu.purpose {
        Purpose::Routing { notes, .. } => notes.as_slice(),
        _ => &[],
    };
    if let Some(empty) = empty {
        let mut rows = vec![(panel::indent(Line::new(empty, MUTED), columns), Part::Keep)];
        rows.extend(notes.iter().map(|note| note_row(note, columns)));
        return rows;
    }
    // Panel-wide notes follow the list within the same room.
    let notes = if room > notes.len() + 1 { notes } else { &[] };
    let limit = room - notes.len();
    let total = menu.filtered.len();
    let start = panel::start(menu.selected, total, limit);
    let table = match &menu.purpose {
        Purpose::Routing { entries, .. } => Some((
            entries,
            routing::Columns::new(entries, table_width(menu, columns)),
        )),
        _ => None,
    };
    let names = name_width(menu, panel::text_width(columns));
    let shown: Vec<_> = menu
        .filtered
        .iter()
        .enumerate()
        .skip(start)
        .take(limit)
        .map(|(index, hit)| {
            let on = index == menu.selected;
            let content = match &table {
                Some((entries, table)) => {
                    let width = table_width(menu, columns);
                    let option = &menu.options[hit.index];
                    let row = panel::fit(table.row(&entries[hit.index], width), width, None);
                    panel::option(
                        row,
                        badge(option.badge.as_ref()),
                        on,
                        false,
                        columns.saturating_sub(2),
                    )
                }
                None => option(menu, hit, on, names, columns),
            };
            panel::choice(content, on, columns)
        })
        .collect();
    let mut rows: Vec<(Line, Part)> = panel::window(shown, start, total, limit, columns)
        .into_iter()
        .enumerate()
        .map(|(offset, line)| {
            let index = start + offset;
            let part = if index == menu.selected {
                Part::Keep
            } else {
                Part::Row(index.abs_diff(menu.selected))
            };
            (line, part)
        })
        .collect();
    rows.extend(notes.iter().map(|note| note_row(note, columns)));
    rows
}

fn note_row(note: &str, columns: usize) -> (Line, Part) {
    (
        panel::indent(Line::new(&text::clean(note), MUTED), columns),
        Part::Header,
    )
}

/// The fact column: the widest name with a fact, plus two, at most 3/5 of
/// the text width.
fn name_width(menu: &Selector, width: usize) -> usize {
    let widest = menu
        .options
        .iter()
        .filter(|option| !option.fact.is_empty())
        .map(|option| text::cells(&text::clean(&option.name)))
        .max();
    widest.map_or(0, |widest| (widest + 2).min(width * 3 / 5))
}

/// `text` with the characters at `matched` in bold accent.
pub(super) fn highlighted(line: &mut Line, value: &str, matched: &[usize], style: Style) {
    for (position, character) in value.chars().enumerate() {
        line.push(
            &character.to_string(),
            if matched.contains(&position) {
                ACCENT.bold()
            } else {
                style
            },
        );
    }
}

/// The marker, the name at column 2 with its match, the fact in its column
/// and the badge ending two cells before the edge.
fn option(menu: &Selector, hit: &Hit, on: bool, names: usize, columns: usize) -> Line {
    let value = &menu.options[hit.index];
    let deleting = menu.delete_target() == Some(hit.index);
    let end = columns.saturating_sub(2);
    let mut line = Line::default();
    let style = if deleting { BAD } else { STRONG };
    // The badge leaves before the marker and a few cells of the name.
    let right = Some(badge(value.badge.as_ref()))
        .filter(|right| end >= 2 + 4 + 2 + panel::cells(right))
        .unwrap_or_default();
    let badge = panel::cells(&right);
    let room = end.saturating_sub(2 + badge + if badge > 0 { 2 } else { 0 });
    let name = text::ellipsize(&text::clean(&value.name), room);
    highlighted(&mut line, &name, &hit.name, style);
    let fact = text::clean(&value.fact);
    let column = names.max(text::cells(&name) + 2);
    // Saved settings read as values; other facts describe the option.
    let tone = match &menu.purpose {
        Purpose::Settings | Purpose::Context => BODY,
        Purpose::Mcp(McpPanel::List) => match value.choice {
            Choice::Mcp(_, Some(health)) => health.style(),
            _ => MUTED,
        },
        Purpose::Lsp(LspPanel::List) => match value.fact.as_str() {
            "Ready" | "Starting" | "Request in progress" => LIT,
            "Executable not found"
            | "Check failed"
            | "Server exited"
            | "Automatic restart paused" => BAD,
            _ => MUTED,
        },
        _ => MUTED,
    };
    // A fact shows only with room for a few of its cells.
    if !fact.is_empty() && room >= column + 6 {
        line = panel::fit(line, column, None);
        let fact = match &menu.purpose {
            Purpose::Remembered { display, .. } => grant_fact(&display[hit.index], room - column),
            _ => text::ellipsize(&fact, room - column),
        };
        highlighted(&mut line, &fact, &hit.fact, tone);
    }
    panel::option(line, right, on, deleting, end)
}

/// A path shortened from the left so the file name stays; a command whole,
/// with its folder shortened.
fn grant_fact(grant: &crate::permissions::Grant, room: usize) -> String {
    let target = text::clean(&grant.target);
    if grant.kind != crate::permissions::Kind::Runs {
        return text::ellipsize_start(&super::footer::path(&target), room);
    }
    let Some(place) = &grant.place else {
        return text::ellipsize(&target, room);
    };
    let rest = room.saturating_sub(text::cells(&target) + 4);
    if rest < 4 {
        return text::ellipsize(&target, room);
    }
    let place = text::ellipsize_start(&super::footer::path(place), rest);
    format!("{target} in {place}")
}

pub(super) fn badge(badge: Option<&Badge>) -> Line {
    let mut line = Line::default();
    match badge {
        Some(Badge::Current) => line.push("current", ACCENT),
        Some(Badge::Default) => line.push("default", MUTED),
        Some(Badge::Note(note)) => line.push(note, MUTED),
        None => {}
    }
    line
}
