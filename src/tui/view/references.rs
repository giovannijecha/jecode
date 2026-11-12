//! The skill and file sheets: the commands sheet's anatomy with their own title,
//! counts, empty states and keys. Rows come from the shared option list; a file row
//! keeps its name in view by shortening its folder from the left.

use super::{
    menus,
    panel::{self, Key, key},
};
use crate::tui::{
    line::Line,
    references::{Browser, Kind, SEARCHED},
    selector::{Choice, Hit, Purpose, Selector},
    text,
    theme::{ACCENT, MUTED, STRONG, Style, WARNING},
};

/// `Files · workspace/src/tui`, then the location shortened from the left, then `Files`.
pub(super) fn title(browser: &Browser, columns: usize) -> Vec<Line> {
    let location = browser.location();
    let mut titles = panel::title(None, "Files", &location);
    let room = columns.saturating_sub(14);
    if room >= 8 && text::cells(&location) > room {
        let short = text::ellipsize_start(&location, room);
        titles.insert(1, panel::title(None, "Files", &short).remove(0));
    }
    titles
}

pub(super) fn aside(menu: &Selector) -> String {
    let Purpose::Files(browser) = &menu.purpose else {
        return String::new();
    };
    if browser.children.is_none() && browser.error.is_none() {
        return "reading…".into();
    }
    let shown = menu.filtered.len();
    if menu.editor.text.trim().is_empty() {
        return text::counted(shown, "item");
    }
    let mut aside = if shown == 1 {
        "1 match".to_string()
    } else {
        format!("{shown} matches")
    };
    if browser.searching {
        aside.push_str(" · searching");
    }
    aside
}

/// What the body says instead of rows.
pub(super) fn empty(menu: &Selector) -> Option<(String, Style)> {
    if !menu.filtered.is_empty() {
        return None;
    }
    let filtering = !menu.editor.text.trim().is_empty();
    let message = match &menu.purpose {
        Purpose::Skills { notes, .. } if menu.options.is_empty() => {
            if notes.is_empty() {
                "No skills in ~/.jecode/skills"
            } else {
                "No valid skills"
            }
        }
        Purpose::Skills { .. } => "No matching skill",
        Purpose::Files(browser) => {
            if let Some(error) = &browser.error {
                return Some((text::clean(error), WARNING));
            }
            if browser.children.is_none() {
                "Reading the folder…"
            } else if filtering && browser.searching {
                "Searching…"
            } else if filtering {
                "No matching file"
            } else {
                "Empty folder"
            }
        }
        _ => return None,
    };
    Some((message.into(), MUTED))
}

/// Muted rows after the list.
pub(super) fn notes(menu: &Selector) -> Vec<String> {
    match &menu.purpose {
        Purpose::Skills { notes, .. } => notes.clone(),
        Purpose::Files(browser) if browser.capped && !menu.editor.text.trim().is_empty() => {
            vec![format!(
                "Searched the first {SEARCHED} entries · open a folder to narrow"
            )]
        }
        _ => Vec::new(),
    }
}

/// Enter names what it does to the focused row; with no row it is absent, because it
/// does nothing.
pub(super) fn keys(menu: &Selector, width: usize) -> Vec<Line> {
    let mut keys: Vec<Key> = Vec::new();
    let focused = match menu.chosen() {
        Some(Choice::Reference(index)) => Some(index),
        _ => None,
    };
    match (&menu.purpose, focused) {
        (Purpose::Files(browser), focused) => {
            if let Some(entry) = focused.and_then(|index| browser.rows.get(index)) {
                let action = if entry.kind == Kind::Directory {
                    "open"
                } else {
                    "insert"
                };
                keys.extend([key("↑↓", "move").optional(), key("Enter", action)]);
            }
            if menu.editor.text.is_empty() && !browser.directory.is_empty() {
                keys.push(key("Backspace", "parent").optional());
            }
        }
        (_, Some(_)) => keys.extend([key("↑↓", "move").optional(), key("Enter", "insert")]),
        (_, None) => {}
    }
    keys.push(key("Esc", "cancel"));
    panel::keys(&keys, width, true)
}

/// The marker, the path with its folder in `muted` and its name in `strong`, the match
/// in bold accent, and a `link` note on the right.
pub(super) fn file_row(menu: &Selector, hit: &Hit, on: bool, columns: usize) -> Line {
    let value = &menu.options[hit.index];
    let end = columns.saturating_sub(2);
    let right = Some(menus::badge(value.badge.as_ref()))
        .filter(|right| end >= 2 + 4 + 2 + panel::cells(right))
        .unwrap_or_default();
    let badge = panel::cells(&right);
    let room = end.saturating_sub(2 + badge + if badge > 0 { 2 } else { 0 });
    let name = text::clean(&value.name);
    let shown = text::ellipsize_start(&name, room);
    // Characters the shortening dropped, replaced by its one `…`.
    let dropped = if shown == name {
        0
    } else {
        name.chars().count() + 1 - shown.chars().count()
    };
    let leaf = shown
        .strip_suffix('/')
        .unwrap_or(&shown)
        .rfind('/')
        .map_or(0, |at| shown[..=at].chars().count());
    let mut line = Line::default();
    for (index, character) in shown.chars().enumerate() {
        let source =
            (index > 0 || dropped == 0).then(|| index + dropped - usize::from(dropped > 0));
        let style = if source.is_some_and(|source| hit.name.contains(&source)) {
            ACCENT.bold()
        } else if index < leaf {
            MUTED
        } else {
            STRONG
        };
        line.push(&character.to_string(), style);
    }
    panel::option(line, right, on, false, end)
}
