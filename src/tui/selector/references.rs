//! The skill and file selectors. They reuse the sheet's rows, filter, matching and
//! selection; they are always filterable, so digits and short catalogs type into the
//! filter instead of choosing.

use super::{Badge, Choice, Hit, OptionRow, Purpose, Selector, row};
use crate::{
    session::commands::match_text,
    skills::SkillInfo,
    tui::references::{Browser, Entry, Kind, Origin, syntax},
};
use std::collections::HashSet;

impl Selector {
    /// The catalog's names with their descriptions as the fact column; the filter matches
    /// both, as every sheet does. `notes` explain what the catalog skipped.
    pub fn skills(origin: Origin, skills: &[SkillInfo], notes: Vec<String>) -> Self {
        let options = skills
            .iter()
            .enumerate()
            .map(|(index, skill)| {
                row(
                    skill.name.clone(),
                    skill.description.clone(),
                    Choice::Reference(index),
                )
            })
            .collect();
        let mut menu = Self::new("Skills", Purpose::Skills { origin, notes }, options);
        menu.searchable = true;
        menu
    }

    pub fn files(browser: Browser) -> Self {
        let mut menu = Self::new("Files", Purpose::Files(Box::new(browser)), vec![]);
        menu.searchable = true;
        menu
    }

    /// The path of the focused file row.
    fn focused_path(&self) -> Option<String> {
        let Purpose::Files(browser) = &self.purpose else {
            return None;
        };
        let hit = self.filtered.get(self.selected)?;
        browser.rows.get(hit.index).map(|entry| entry.path.clone())
    }

    /// Rows from what the browser holds: the folder's children, or with a filter its
    /// subtree ranked by name, then by path. A typed path to an existing entry leads.
    /// `keep` holds the focus on the same entry while results arrive; a new filter
    /// focuses the best match.
    pub fn refresh_files(&mut self, keep: bool) {
        let previous = keep.then(|| self.focused_path()).flatten();
        let Purpose::Files(browser) = &mut self.purpose else {
            return;
        };
        let query = syntax::typed_path(&self.editor.text);
        let filtering = !query.is_empty();
        let prefix = if browser.directory.is_empty() {
            String::new()
        } else {
            format!("{}/", browser.directory)
        };
        let shown = |entry: &Entry| {
            let mut shown = entry
                .path
                .strip_prefix(&prefix)
                .unwrap_or(&entry.path)
                .to_string();
            if entry.kind == Kind::Directory {
                shown.push('/');
            }
            shown
        };
        let explicit = filtering
            .then(|| browser.explicit(&self.editor.text))
            .flatten();
        let children = browser.children.as_deref().unwrap_or(&[]);
        let subtree = if filtering {
            browser.subtree.as_slice()
        } else {
            &[]
        };
        let mut ranked: Vec<(usize, usize, Entry, String, Vec<usize>)> = Vec::new();
        if let Some(entry) = explicit {
            let name = shown(&entry);
            ranked.push((0, 0, entry, name, Vec::new()));
        }
        // Direct children remain available even when a partial subtree stops at
        // the search cap before its walk reaches a sibling of a deep folder.
        let mut seen = HashSet::new();
        for (order, entry) in children.iter().chain(subtree).enumerate() {
            if !seen.insert(entry.path.as_str()) {
                continue;
            }
            if ranked
                .first()
                .is_some_and(|(_, _, first, ..)| first.path == entry.path)
            {
                continue;
            }
            let name = shown(entry);
            let (score, positions) = if filtering {
                match rank(&query, &name) {
                    Some(found) => found,
                    None => continue,
                }
            } else {
                (0, Vec::new())
            };
            ranked.push((score, order + 1, entry.clone(), name, positions));
        }
        ranked.sort_by_key(|(score, order, ..)| (*score, *order));
        browser.rows.clear();
        self.options.clear();
        self.filtered.clear();
        for (index, (_, _, entry, name, positions)) in ranked.into_iter().enumerate() {
            let badge = match entry.kind {
                Kind::Link => Some(Badge::Note("link".into())),
                Kind::Other => Some(Badge::Note("other".into())),
                Kind::Directory | Kind::File => None,
            };
            self.options.push(OptionRow {
                badge,
                ..row(name, "", Choice::Reference(index))
            });
            self.filtered.push(Hit {
                index,
                name: positions,
                fact: Vec::new(),
            });
            browser.rows.push(entry);
        }
        self.context = browser.location();
        let wanted = browser.focus.clone().or(previous);
        let found = wanted
            .as_ref()
            .and_then(|path| browser.rows.iter().position(|entry| &entry.path == path));
        self.selected = found.unwrap_or(0);
        // A focus waits for its entry until the walk has delivered everything.
        if found.is_some() || !browser.pending() {
            browser.focus = None;
        }
    }
}

/// A name match ranks first and highlights within the name; a path match, or any
/// query with a separator, matches the whole relative path.
fn rank(query: &str, shown: &str) -> Option<(usize, Vec<usize>)> {
    if query.contains('/') {
        return match_text(query, shown);
    }
    let trimmed = shown.strip_suffix('/').unwrap_or(shown);
    let start = trimmed.rfind('/').map_or(0, |at| at + 1);
    let offset = shown[..start].chars().count();
    if let Some((score, positions)) = match_text(query, &shown[start..]) {
        return Some((
            score,
            positions
                .into_iter()
                .map(|position| position + offset)
                .collect(),
        ));
    }
    match_text(query, shown).map(|(score, positions)| (score + 10_000, positions))
}
