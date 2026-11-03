//! Global language-server settings and read-only conversation inspection.
mod fields;
mod flow;
mod job;
mod menus;
#[cfg(test)]
mod tests;

use super::{
    App,
    selector::{Purpose, Selector, Setting},
};
use crate::lsp::{self, Profile, Store};
use job::CheckJob;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Field {
    Name,
    Executable,
    Languages,
    Arguments,
    Environment,
    RootMarkers,
    RootOverride,
    Initialization,
    Settings,
    Timeout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Panel {
    List,
    Add,
    Detail,
    Form,
    Advanced,
    Field(Field),
    Checking,
    ConfirmRemove,
    Instance,
}

#[derive(Default)]
pub(super) struct Ui {
    settings: bool,
    from_inspection: bool,
    profiles: Vec<Profile>,
    observed: Vec<lsp::Status>,
    selected: Option<String>,
    draft: Option<Profile>,
    original: Option<Profile>,
    check: Option<CheckJob>,
    check_back: Option<Panel>,
}

impl App {
    fn lsp_home(&self) -> PathBuf {
        self.config
            .store
            .path()
            .parent()
            .expect("configuration directory")
            .into()
    }

    fn lsp_store(&self) -> Store {
        Store::new(self.lsp_home())
    }

    pub(super) fn open_lsp(&mut self, settings: bool) {
        self.state.information = None;
        self.lsp = Ui {
            settings,
            ..Ui::default()
        };
        match self.lsp_store().load() {
            Ok(profiles) => {
                for profile in &profiles {
                    profile.protect(&self.archive.redactor);
                }
                self.lsp.profiles = profiles;
                self.lsp.observed = self.facts().lsp_runtime.statuses();
                self.show_lsp_list();
            }
            Err(error) => {
                self.warn(&error);
                self.show_lsp_list();
            }
        }
    }

    fn selected_lsp(&self) -> Option<&Profile> {
        let id = self.lsp.selected.as_deref()?;
        self.lsp.profiles.iter().find(|profile| profile.id == id)
    }

    fn show_lsp_list(&mut self) {
        self.lsp.observed = self.facts().lsp_runtime.statuses();
        let mut menu = menus::list(&self.lsp.profiles, &self.lsp.observed, self.lsp.settings);
        if let Some(index) = self
            .lsp
            .selected
            .as_ref()
            .and_then(|id| self.lsp.profiles.iter().position(|p| &p.id == id))
        {
            menu.selected = index;
        }
        self.state.selector = Some(menu);
    }

    fn show_lsp_add(&mut self) {
        self.state.selector = Some(menus::add());
    }

    fn show_lsp_detail(&mut self) {
        self.lsp.observed = self.facts().lsp_runtime.statuses();
        let Some(profile) = self.selected_lsp() else {
            self.show_lsp_list();
            return;
        };
        let observed = self
            .lsp
            .observed
            .iter()
            .find(|status| status.server == profile.id);
        self.state.selector = Some(menus::detail(profile, observed, self.lsp.settings));
    }

    fn show_lsp_form(&mut self) {
        let Some(profile) = &self.lsp.draft else {
            self.show_lsp_detail();
            return;
        };
        self.state.selector = Some(menus::form(profile));
    }

    fn show_lsp_advanced(&mut self) {
        let Some(profile) = &self.lsp.draft else {
            self.show_lsp_form();
            return;
        };
        self.state.selector = Some(menus::advanced(profile));
    }

    fn show_lsp_remove(&mut self) {
        let name = self
            .selected_lsp()
            .map(|p| p.name.clone())
            .unwrap_or_default();
        self.state.selector = Some(Selector::lsp(
            Panel::ConfirmRemove,
            "Remove server?",
            Some("LSP"),
            &name,
            vec![("Remove".into(), name.clone())],
        ));
    }

    fn show_lsp_instance(&mut self) {
        self.lsp.observed = self.facts().lsp_runtime.statuses();
        let Some(profile) = self.selected_lsp() else {
            self.show_lsp_list();
            return;
        };
        let mut menu = Selector::lsp(
            Panel::Instance,
            &profile.name,
            Some("LSP"),
            "this conversation",
            vec![("Settings detail".into(), String::new())],
        );
        menu.message = self
            .lsp
            .observed
            .iter()
            .find(|status| status.server == profile.id)
            .map_or("Not started".into(), |status| {
                let capabilities = status
                    .capabilities
                    .as_ref()
                    .map_or(String::new(), |capabilities| {
                        format!("\n{}", menus::capability_facts(capabilities))
                    });
                format!(
                    "{} · {}\n{}{}{}",
                    menus::state_label(&status.state),
                    status
                        .root
                        .as_ref()
                        .map_or("No root observed".into(), |root| root.display().to_string()),
                    status.detail,
                    status
                        .pid
                        .map_or(String::new(), |pid| format!("\nProcess {pid}")),
                    capabilities,
                )
            });
        self.state.selector = Some(menu);
    }

    fn begin_lsp_draft(&mut self, mut profile: Profile, saved: bool) {
        if !saved {
            self.lsp.selected = None;
            if !profile.name.is_empty() {
                let base = profile.name.clone();
                let mut suffix = 2;
                while self
                    .lsp
                    .profiles
                    .iter()
                    .any(|existing| existing.name.eq_ignore_ascii_case(&profile.name))
                {
                    profile.name = format!("{base} {suffix}");
                    suffix += 1;
                }
            }
        }
        self.lsp.original = saved.then(|| profile.clone());
        self.lsp.draft = Some(profile);
        self.show_lsp_form();
    }

    pub(super) fn choose_lsp(&mut self, panel: Panel, index: usize) {
        match panel {
            Panel::List => {
                if index == self.lsp.profiles.len() && self.lsp.settings {
                    self.show_lsp_add();
                } else if let Some(profile) = self.lsp.profiles.get(index) {
                    self.lsp.selected = Some(profile.id.clone());
                    if self.lsp.settings {
                        self.show_lsp_detail();
                    } else if self
                        .lsp
                        .observed
                        .iter()
                        .any(|status| status.server == profile.id && status.state == "ready")
                    {
                        self.show_lsp_instance();
                    } else {
                        self.lsp.settings = true;
                        self.lsp.from_inspection = true;
                        self.show_lsp_detail();
                    }
                }
            }
            Panel::Add => {
                let draft = match index {
                    0 => Profile::draft_guided_rust(),
                    1 => Profile::draft_guided_cpp(),
                    _ => Profile::draft_custom(),
                };
                self.begin_lsp_draft(draft, false);
            }
            Panel::Detail => match index {
                0 => {
                    if let Some(profile) = self.selected_lsp().cloned() {
                        self.begin_lsp_draft(profile, true);
                    }
                }
                1 => self.toggle_lsp(),
                2 => self.start_lsp_check(false),
                3 if self.lsp.observed.iter().any(|status| {
                    self.lsp.selected.as_deref() == Some(status.server.as_str())
                        && status.state == "restart_suppressed"
                }) =>
                {
                    if let Some(id) = self.lsp.selected.clone() {
                        self.allow_lsp_restart(&id);
                    }
                }
                3 | 4 => self.show_lsp_remove(),
                _ => {}
            },
            Panel::Form => match index {
                0 => self.show_lsp_field(Field::Name),
                1 => self.show_lsp_field(Field::Executable),
                2 => self.show_lsp_field(Field::Languages),
                3 => {
                    if let Some(profile) = &mut self.lsp.draft {
                        profile.enabled = !profile.enabled;
                    }
                    self.show_lsp_form();
                }
                4 => self.show_lsp_advanced(),
                5 => self.start_lsp_check(true),
                6 => self.save_lsp_draft(),
                _ => {}
            },
            Panel::Advanced => {
                let fields = [
                    Field::Arguments,
                    Field::Environment,
                    Field::RootMarkers,
                    Field::RootOverride,
                    Field::Initialization,
                    Field::Settings,
                    Field::Timeout,
                ];
                if let Some(field) = fields.get(index) {
                    self.show_lsp_field(*field);
                }
            }
            Panel::ConfirmRemove if index == 0 => self.remove_lsp(),
            Panel::Instance if index == 0 => {
                self.lsp.settings = true;
                self.lsp.from_inspection = true;
                self.show_lsp_detail();
            }
            _ => {}
        }
    }

    fn show_lsp_back(&mut self, panel: Panel) {
        match panel {
            Panel::Form => self.show_lsp_form(),
            Panel::Detail => self.show_lsp_detail(),
            _ => self.show_lsp_list(),
        }
    }

    pub(in crate::tui) fn close_lsp_panel(&mut self) -> bool {
        let panel = match self.state.selector.as_ref().map(|menu| &menu.purpose) {
            Some(Purpose::Lsp(panel)) => *panel,
            _ => return false,
        };
        match panel {
            Panel::List => {
                if self.lsp.from_inspection {
                    self.lsp.settings = false;
                    self.lsp.from_inspection = false;
                    self.show_lsp_list();
                    return true;
                }
                let settings = self.lsp.settings;
                self.lsp = Ui::default();
                if settings {
                    self.settings_parent = Some(Setting::Lsp);
                    self.return_to_settings();
                } else {
                    self.state.selector = None;
                }
            }
            Panel::Add => self.show_lsp_list(),
            Panel::Detail if self.lsp.from_inspection => {
                self.lsp.settings = false;
                self.lsp.from_inspection = false;
                self.show_lsp_instance();
            }
            Panel::Detail | Panel::Instance => self.show_lsp_list(),
            Panel::Form => {
                self.lsp.draft = None;
                self.lsp.original = None;
                if self.lsp.selected.is_some() {
                    self.show_lsp_detail();
                } else {
                    self.show_lsp_add();
                }
            }
            Panel::Advanced | Panel::Field(Field::Name | Field::Executable | Field::Languages) => {
                self.show_lsp_form()
            }
            Panel::Field(_) => self.show_lsp_advanced(),
            Panel::ConfirmRemove => self.show_lsp_detail(),
            Panel::Checking => {
                self.lsp.check.take();
                let back = self.lsp.check_back.take().unwrap_or(Panel::Form);
                self.show_lsp_back(back);
            }
        }
        true
    }

    pub(in crate::tui) fn lsp_empty_enter(&mut self) -> bool {
        if self
            .state
            .selector
            .as_ref()
            .is_some_and(|menu| matches!(menu.purpose, Purpose::Lsp(Panel::List)))
            && !self.lsp.settings
            && self.lsp.profiles.is_empty()
        {
            self.lsp.settings = true;
            self.lsp.from_inspection = true;
            self.show_lsp_list();
            return true;
        }
        false
    }
}
