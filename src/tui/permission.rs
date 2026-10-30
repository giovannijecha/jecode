//! Permissions in the TUI (B12): the request in the composer's place, the
//! `/permissions` menus, Settings › Default permissions and the level beside
//! the draft. The state is the agent's, shared, so every change applies at
//! once, during a turn too: the next call follows it.

use super::{
    App, Feedback, Kind,
    arrival::{self, Guard},
    selector::{Choice, Purpose, Selector},
    terminal::Key,
    viewport::Scroll,
};
use crate::{
    json::Value,
    permissions::{Decision, Grant, Kind as Access, Level, Request, wording},
};
use std::time::Instant;

/// The request shown, with who asks as the title badge reads.
#[derive(Clone, PartialEq, Debug)]
pub struct Pending {
    pub request: Request,
    pub waiting: usize,
    /// `MAIN AGENT` or `AGENT N`.
    pub agent: String,
    /// No row is focused on arrival, so Enter alone decides nothing (B12.6 T2).
    pub focus: Option<usize>,
    /// Arrived while the draft was typed in: until the keys pause or an
    /// arrow moves, typing continues the draft and no digit decides.
    pub guard: Option<Guard>,
}

const DECISIONS: [Decision; 3] = [Decision::Once, Decision::Remember, Decision::Deny];

impl App {
    /// Copies the level and the front of the request queue into the view.
    pub(super) fn refresh_permission(&mut self) -> bool {
        let level = self.permission_observer.level();
        let mut dirty = level != self.state.permission_level;
        self.state.permission_level = level;
        let pending = self
            .permission_observer
            .pending()
            .map(|(mut request, count)| {
                request.access.target = self.archive.redactor.text(&request.access.target);
                request.access.place = request
                    .access
                    .place
                    .map(|place| self.archive.redactor.text(&place));
                (request, count)
            });
        let same = match (&pending, &self.state.permission) {
            (Some((request, waiting)), Some(shown)) => {
                shown.request == *request && shown.waiting == *waiting
            }
            (None, None) => true,
            _ => false,
        };
        if !same {
            dirty = true;
            let now = Instant::now();
            self.state.permission = pending.map(|(request, waiting)| {
                let shown = self
                    .state
                    .permission
                    .as_ref()
                    .filter(|shown| shown.request == request);
                let focus = shown.and_then(|shown| shown.focus);
                let guard = match shown {
                    Some(shown) => shown.guard,
                    None => Guard::arrive(self.typed, now),
                };
                let agent = self.asking_agent(request.agent.as_deref());
                Pending {
                    request,
                    waiting,
                    agent,
                    focus,
                    guard,
                }
            });
            // A request closes any menu over it, as a question does.
            if self.state.permission.is_some() {
                self.make_room_for_ask();
            }
        }
        if dirty {
            self.state.changed();
        }
        dirty
    }

    fn asking_agent(&self, id: Option<&str>) -> String {
        let Some(id) = id else {
            return "MAIN AGENT".into();
        };
        self.state
            .collaboration
            .iter()
            .find(|record| record.get("id").and_then(Value::as_str) == Some(id))
            .map_or_else(
                || format!("AGENT {}", id.trim_start_matches('a')),
                super::agents::label,
            )
    }

    /// 1–3 choose at once; letters reach neither the request nor the draft,
    /// except while the typing the request arrived into goes on.
    pub(super) fn permission_text(&mut self, text: &str) {
        if self.permission_guarded() {
            self.continue_draft(text);
            return;
        }
        let mut characters = text.chars();
        if let (Some(digit @ '1'..='3'), None) = (characters.next(), characters.next()) {
            self.decide_permission(digit as usize - '1' as usize);
        }
    }

    /// The arrival guard holds for this key; an expired guard is cleared.
    pub(super) fn permission_guarded(&mut self) -> bool {
        let Some(shown) = self.state.permission.as_mut() else {
            return false;
        };
        let holds = shown
            .guard
            .as_mut()
            .is_some_and(|guard| guard.holds(Instant::now()));
        if !holds {
            shown.guard = None;
        }
        holds
    }

    /// The request keeps the keys until a choice; Ctrl+C stops the turn.
    pub(super) fn permission_key(&mut self, key: Key) {
        if key.ctrl() && key.code == 67 {
            self.stop();
            return;
        }
        let plain = !key.ctrl() && !key.alt() && !key.shift();
        if arrival::navigation(key.code) {
            if let Some(shown) = self.state.permission.as_mut() {
                shown.guard = None;
            }
        } else if !matches!(key.code, 33 | 34) && self.permission_guarded() {
            // Enter decides nothing; editing keys keep the draft.
            if !(key.code == 13 && plain) {
                self.edit_draft(key);
            }
            return;
        }
        let Some(shown) = self.state.permission.as_mut() else {
            return;
        };
        match key.code {
            38 | 40 if plain => {
                shown.focus = Some(match shown.focus {
                    None => 0,
                    Some(focus) if key.code == 38 => focus.saturating_sub(1),
                    Some(focus) => (focus + 1).min(DECISIONS.len() - 1),
                });
                self.state.changed();
            }
            13 if plain => {
                if let Some(focus) = shown.focus {
                    self.decide_permission(focus);
                }
            }
            33 | 34 if !key.ctrl() && !key.alt() => {
                self.renderer.scroll(Scroll::Page(key.code == 33));
            }
            _ => {}
        }
    }

    fn decide_permission(&mut self, choice: usize) {
        let Some(shown) = self.state.permission.take() else {
            return;
        };
        let decision = DECISIONS[choice];
        match self.permission_observer.decide(shown.request.id, decision) {
            Err(error) => self.warn(&error),
            Ok(()) if decision == Decision::Remember => self.permission_notice(&format!(
                "Permission: {} · remembered for this conversation",
                wording::covers(&shown.request.access, &shown.request.tool)
            )),
            Ok(()) => {}
        }
        self.refresh_permission();
        self.state.changed();
    }

    fn permission_notice(&mut self, text: &str) {
        self.state
            .notify(Feedback::result(Kind::Notice, self.archive.redactor.text(text)).change());
    }

    /// `/permissions`, or `/permissions LEVEL` applied at once.
    pub(super) fn open_permissions(&mut self, arguments: &str) {
        if arguments.is_empty() {
            self.state.selector = Some(self.permissions_menu());
            return;
        }
        match Level::parse(&arguments.to_ascii_lowercase()) {
            Some(level) => self.apply_level(level),
            None => self.warn("Usage: /permissions [read|workspace|full]"),
        }
    }

    fn permissions_menu(&self) -> Selector {
        let permissions = &self.permission_observer;
        Selector::permissions(
            permissions.level(),
            permissions.default_level(),
            permissions.grants().len(),
        )
    }

    pub(super) fn choose_permissions(&mut self, choice: Choice, menu: Selector) {
        match (choice, &menu.purpose) {
            (Choice::Level(level), Purpose::Permissions { .. }) => self.apply_level(level),
            (Choice::Remembered, Purpose::Permissions { .. }) => self.open_remembered(0),
            (Choice::Level(level), Purpose::DefaultPermissions) => self.save_default_level(level),
            _ => self.state.selector = Some(menu),
        }
    }

    /// Esc: Remembered returns to its row in `/permissions`; the level menu
    /// closes with no notice. `false` for other menus.
    pub(super) fn close_permissions_menu(&mut self) -> bool {
        match self.state.selector.as_ref().map(|menu| &menu.purpose) {
            Some(Purpose::Permissions { .. }) => self.state.selector = None,
            Some(Purpose::Remembered { .. }) => {
                let mut menu = self.permissions_menu();
                menu.selected = Level::ALL.len();
                self.state.selector = Some(menu);
            }
            _ => return false,
        }
        true
    }

    /// Calls already running keep their level; the next call follows this.
    fn apply_level(&mut self, level: Level) {
        let previous = self.permission_observer.level();
        if level == previous {
            return;
        }
        if let Err(error) = self.permission_observer.set_level(level) {
            self.state.notify(Feedback::result(
                Kind::Error,
                self.archive.redactor.text(&error),
            ));
            return;
        }
        let processes = self.process_observer.processes().map_or(0, |processes| {
            processes.iter().filter(|process| process.active()).count()
        });
        let fact = match processes {
            0 if self.held.is_some() => "from the next call".into(),
            0 => "this conversation".into(),
            1 => format!("1 process keeps {}", previous.name()),
            count => format!("{count} processes keep {}", previous.name()),
        };
        self.permission_notice(&format!(
            "Permissions: {} → {} · {fact}",
            previous.name(),
            level.name()
        ));
        self.refresh_permission();
    }

    fn open_remembered(&mut self, selected: usize) {
        let permissions = &self.permission_observer;
        let mut menu = Selector::remembered(permissions.grants(), permissions.level());
        for option in &mut menu.options {
            option.name = self.archive.redactor.text(&option.name);
            option.fact = self.archive.redactor.text(&option.fact);
        }
        if let Purpose::Remembered { display, .. } = &mut menu.purpose {
            for grant in display {
                grant.target = self.archive.redactor.text(&grant.target);
                grant.place = grant
                    .place
                    .take()
                    .map(|place| self.archive.redactor.text(&place));
            }
        }
        menu.selected = selected.min(menu.filtered.len().saturating_sub(1));
        self.state.selector = Some(menu);
    }

    /// Ctrl+D in Remembered revokes at once; the next such call asks again.
    pub(super) fn revoke_grant(&mut self) -> bool {
        let Some(menu) = &self.state.selector else {
            return false;
        };
        let Purpose::Remembered { grants, .. } = &menu.purpose else {
            return false;
        };
        let Some(Choice::Grant(index)) = menu.chosen() else {
            return true;
        };
        let (grant, selected) = (grants[index].clone(), menu.selected);
        match self.permission_observer.revoke(&grant) {
            Ok(()) => {
                self.permission_notice(&format!("Revoked: {}", revoked(&grant)));
                self.open_remembered(selected);
            }
            Err(error) => self.warn(&error),
        }
        true
    }

    /// Saves the default for new conversations; this one keeps its level.
    fn save_default_level(&mut self, level: Level) {
        let previous = self.config.settings.default_permissions;
        if level == previous {
            self.return_to_settings();
            return;
        }
        let mut settings = self.config.settings.clone();
        settings.default_permissions = level;
        let first_setup = !settings.has_model();
        let saved = if !first_setup {
            self.config.store.save(&settings)
        } else {
            Ok(())
        };
        if let Err(error) = saved {
            self.finish_setting(Kind::Error, &error, vec![]);
            return;
        }
        self.config.settings = settings;
        self.permission_observer.set_default(level);
        let current = self.permission_observer.level();
        let fact = if first_setup {
            "this session · choose a default model to save".into()
        } else if current == level {
            "new conversations".into()
        } else {
            format!("this conversation stays {}", current.name())
        };
        self.finish_setting_change(
            Kind::Notice,
            &format!(
                "Default permissions: {} → {} · {fact}",
                previous.name(),
                level.name()
            ),
            vec![],
        );
    }

    /// `Workspace · 2 remembered grants`, for the resumed notice (B12.6 T7).
    pub(super) fn permission_summary(&self) -> String {
        let grants = self.permission_observer.grants().len();
        let level = self.permission_observer.level().name();
        match grants {
            0 => level.into(),
            1 => format!("{level} · 1 remembered grant"),
            count => format!("{level} · {count} remembered grants"),
        }
    }
}

/// `session.rs · reads ask again`, or the command for a run.
fn revoked(grant: &Grant) -> String {
    let (name, kind) = match grant.kind {
        Access::Reads => (file_name(&grant.target), "reads"),
        Access::Edits => (file_name(&grant.target), "edits"),
        Access::Runs => (grant.target.as_str(), "runs"),
    };
    format!("{name} · {kind} ask again")
}

fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/'])
        .find(|part| !part.is_empty())
        .unwrap_or(path)
}

#[cfg(test)]
mod tests;
