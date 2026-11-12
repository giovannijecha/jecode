//! Explicit references typed in the composer: `$` chooses a personal skill and `@` a
//! workspace file. Each opens a sheet with its own filter over the kept draft; choosing
//! writes the reference's text where the sigil or the replaced reference stood and
//! returns to the draft. A reference is text only: the agent reads what it names
//! through its usual tools, when it needs to.

mod files;
pub mod syntax;

pub(super) use files::{Browser, Entry, Kind, SEARCHED};

use super::{
    App,
    editor::Editor,
    selector::{Choice, Purpose, Selector},
    terminal::Key,
};
use syntax::Sigil;

/// Where a chosen reference goes: the draft as it stood when the selector opened, and
/// the byte range to replace, the typed sigil or an existing reference.
#[derive(Debug)]
pub struct Origin {
    pub draft: Editor,
    pub start: usize,
    pub end: usize,
}

impl App {
    /// A sigil typed in the composer goes into the draft like any character; at a word
    /// boundary it also opens its selector, with the sigil as the range to replace.
    pub(super) fn type_reference(&mut self, text: &str) -> bool {
        let Some(sigil) = Sigil::of(text) else {
            return false;
        };
        let editor = &self.state.editor;
        if editor.text.starts_with('/') || !syntax::opens(&editor.text, editor.cursor) {
            return false;
        }
        self.continue_draft(text);
        let end = self.state.editor.cursor;
        if self.state.editor.text[..end].ends_with(sigil.character()) {
            self.open_reference(sigil, end - 1, end, "");
        }
        true
    }

    /// Tab on a reference in the composer reopens its selector to replace it.
    pub(super) fn retarget_reference(&mut self) -> bool {
        let editor = &self.state.editor;
        if editor.text.starts_with('/') {
            return false;
        }
        let Some(token) = syntax::at(&editor.text, editor.cursor) else {
            return false;
        };
        let filter = match token.sigil {
            Sigil::Skill => token.value,
            Sigil::File => syntax::file(&token.value).unwrap_or(token.value),
        };
        self.open_reference(token.sigil, token.start, token.end, &filter);
        true
    }

    fn open_reference(&mut self, sigil: Sigil, start: usize, end: usize, filter: &str) {
        self.state.clear_notice();
        self.state.suggestions.dismiss();
        let origin = Origin {
            draft: self.state.editor.clone(),
            start,
            end,
        };
        let mut menu = match sigil {
            Sigil::Skill => match self.skill_catalog() {
                Ok((skills, warnings)) => {
                    let notes = if warnings.is_empty() {
                        Vec::new()
                    } else {
                        vec![format!("{} skipped · /skills shows why", warnings.len())]
                    };
                    Selector::skills(origin, &skills, notes)
                }
                Err(error) => Selector::skills(origin, &[], vec![error]),
            },
            Sigil::File => {
                let session = self.persistence.as_ref().map(|handle| handle.id());
                let root = self.archive.directory.clone();
                Selector::files(Browser::new(origin, root, session))
            }
        };
        if !filter.is_empty() && menu.editor.insert(filter) {
            menu.refresh();
        }
        self.state.selector = Some(menu);
    }

    /// The personal skill catalog, read locally: from the agent, or while a turn holds
    /// it, from the same library directory.
    pub(super) fn skill_catalog(
        &mut self,
    ) -> Result<(Vec<crate::skills::SkillInfo>, Vec<String>), String> {
        if let Some(agent) = &mut self.agent {
            agent
                .skills()
                .map(|skills| (skills, agent.skill_warnings().to_vec()))
        } else {
            let mut library = crate::skills::Library::at(self.facts().skills.clone());
            library.refresh();
            Ok((library.list(), library.warnings().to_vec()))
        }
    }

    /// The keys a skill or file selector handles itself; others fall to the shared ones.
    pub(super) fn reference_key(&mut self, key: Key) -> bool {
        let plain = !key.ctrl() && !key.alt() && !key.shift();
        let Some(menu) = &mut self.state.selector else {
            return false;
        };
        match (key.code, &mut menu.purpose) {
            (13 | 9, Purpose::Skills { .. } | Purpose::Files(_)) if plain => {
                self.choose_reference();
                true
            }
            (8, Purpose::Files(browser)) if plain && menu.editor.text.is_empty() => {
                if let Some(filter) = browser.leave() {
                    menu.editor = filter;
                    menu.refresh();
                }
                true
            }
            _ => false,
        }
    }

    /// Enter or Tab: a skill or file is written into the draft, a folder opens. With
    /// nothing selected nothing happens, and nothing is ever sent.
    fn choose_reference(&mut self) {
        let Some(menu) = self.state.selector.as_mut() else {
            return;
        };
        let Some(Choice::Reference(index)) = menu.chosen() else {
            return;
        };
        let chosen = match &mut menu.purpose {
            Purpose::Skills { .. } => Ok(syntax::skill(&menu.options[index].name)),
            Purpose::Files(browser) => {
                let entry = browser.rows[index].clone();
                if !browser.exists(&entry.path) {
                    browser.load();
                    menu.refresh();
                    Err(format!("{} no longer exists", entry.path))
                } else if entry.kind == Kind::Directory {
                    let filter = std::mem::take(&mut menu.editor);
                    browser.enter(&entry.path, &filter, Some(entry.path.clone()));
                    menu.refresh();
                    return;
                } else {
                    syntax::file(&entry.path).ok_or_else(|| {
                        "This file name contains characters that cannot be referenced".into()
                    })
                }
            }
            _ => return,
        };
        let reference = match chosen {
            Ok(reference) => reference,
            Err(warning) => return self.warn(&warning),
        };
        let origin = match self.state.selector.take().map(|menu| menu.purpose) {
            Some(Purpose::Skills { origin, .. }) => origin,
            Some(Purpose::Files(browser)) => browser.origin,
            _ => return,
        };
        self.insert_reference(origin, reference);
    }

    /// Writes `reference` over the origin's range, preserving following punctuation
    /// and whitespace. A new reference gains a trailing space where one is needed.
    fn insert_reference(&mut self, origin: Origin, mut reference: String) {
        if self.state.editor != origin.draft {
            self.warn("The draft changed while choosing · nothing inserted");
            return;
        }
        let editor = &mut self.state.editor;
        let suffix = &editor.text[origin.end..];
        let spaced = suffix.starts_with(' ');
        if suffix.is_empty()
            || origin.end == origin.start + 1 && !suffix.starts_with(char::is_whitespace)
        {
            reference.push(' ');
        }
        if !editor.splice(origin.start, origin.end, &reference) {
            self.warn("Input exceeds 1 MiB · insertion rejected, your draft was kept");
            return;
        }
        if spaced {
            editor.cursor += 1;
        }
        self.typed = Some(std::time::Instant::now());
        self.state.feedback_input(true);
        self.edited();
        self.state.history.edited();
    }

    /// Takes what a file walk delivered. A walk from another conversation is stale, so
    /// the selector closes with the draft untouched.
    pub(super) fn poll_references(&mut self) -> bool {
        let session = self.persistence.as_ref().map(|handle| handle.id());
        let Some(menu) = &mut self.state.selector else {
            return false;
        };
        let Purpose::Files(browser) = &mut menu.purpose else {
            return false;
        };
        if browser.session != session {
            self.state.selector = None;
            return true;
        }
        if browser.poll() {
            menu.refresh_files(true);
            return true;
        }
        false
    }

    /// A file walk is still delivering, so the loop polls soon.
    pub(super) fn references_pending(&self) -> bool {
        self.state.selector.as_ref().is_some_and(
            |menu| matches!(&menu.purpose, Purpose::Files(browser) if browser.pending()),
        )
    }
}

#[cfg(test)]
mod app_tests;
#[cfg(test)]
mod tests;
