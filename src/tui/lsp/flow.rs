//! Saved LSP changes and explicit temporary checks.
use super::{App, Panel, job::CheckJob};
use crate::{
    lsp::{CheckFailure, CheckObservation},
    tui::{Feedback, Kind, selector::Selector},
};

impl App {
    pub(super) fn save_lsp_draft(&mut self) {
        let Some(profile) = self.lsp.draft.clone() else {
            return;
        };
        if let Err(error) = profile.validate() {
            self.warn(&error);
            return;
        }
        if self
            .lsp
            .profiles
            .iter()
            .any(|other| other.id != profile.id && other.name.eq_ignore_ascii_case(&profile.name))
        {
            self.warn("A server with this name already exists");
            return;
        }
        let changed = self.lsp.original.as_ref().is_none_or(|original| {
            original.name != profile.name
                || original.enabled != profile.enabled
                || !profile.same_analysis_settings(original)
        });
        match self.lsp_store().save(profile.clone()) {
            Ok(()) => {
                self.lsp.selected = Some(profile.id.clone());
                self.lsp
                    .profiles
                    .retain(|existing| existing.id != profile.id);
                self.lsp.profiles.push(profile.clone());
                self.lsp.draft = None;
                self.lsp.original = None;
                if changed {
                    self.lsp_changed(&profile.id);
                }
                self.lsp_notice("Language server saved · starts on first matching model request");
                self.show_lsp_detail();
            }
            Err(error) => self.warn(&error),
        }
    }

    pub(super) fn toggle_lsp(&mut self) {
        let Some(profile) = self.selected_lsp().cloned() else {
            return;
        };
        match self.lsp_store().set_enabled(&profile.id, !profile.enabled) {
            Ok(()) => {
                if let Some(saved) = self
                    .lsp
                    .profiles
                    .iter_mut()
                    .find(|saved| saved.id == profile.id)
                {
                    saved.enabled = !profile.enabled;
                }
                self.lsp_changed(&profile.id);
                self.lsp_notice(if profile.enabled {
                    "Language server turned off"
                } else {
                    "Language server turned on · starts when needed"
                });
                self.show_lsp_detail();
            }
            Err(error) => self.warn(&error),
        }
    }

    pub(super) fn remove_lsp(&mut self) {
        let Some(id) = self.lsp.selected.clone() else {
            return;
        };
        match self.lsp_store().remove(&id) {
            Ok(()) => {
                self.lsp.profiles.retain(|p| p.id != id);
                self.lsp.selected = None;
                self.lsp_changed(&id);
                self.lsp_notice("Language server removed");
                self.show_lsp_list();
            }
            Err(error) => self.warn(&error),
        }
    }

    fn lsp_changed(&mut self, id: &str) {
        self.facts().lsp_runtime.configuration_changed(id);
    }

    fn lsp_notice(&mut self, message: &str) {
        self.state
            .notify(Feedback::result(Kind::Notice, message).change());
    }

    pub(super) fn start_lsp_check(&mut self, draft: bool) {
        let profile = if draft {
            self.lsp.draft.clone()
        } else {
            self.selected_lsp().cloned()
        };
        let Some(profile) = profile else {
            return;
        };
        if let Err(error) = profile.validate() {
            self.warn(&error);
            return;
        }
        let back = if draft { Panel::Form } else { Panel::Detail };
        self.lsp.check = Some(CheckJob::start(profile, self.archive.directory.clone()));
        self.lsp.check_back = Some(back);
        let mut menu = Selector::lsp(
            Panel::Checking,
            "LSP check",
            Some("LSP"),
            "temporary server",
            vec![],
        );
        menu.message = "Starting, initializing and stopping the selected server…".into();
        self.state.selector = Some(menu);
    }

    pub(in crate::tui) fn poll_lsp(&mut self) -> bool {
        if !self.lsp.check.as_ref().is_some_and(CheckJob::finished) {
            return false;
        }
        let result = self.lsp.check.take().unwrap().finish();
        let back = self.lsp.check_back.take().unwrap_or(Panel::Form);
        match result {
            Ok((id, observation)) => self.finish_lsp_check(&id, observation, back),
            Err(error) => self.finish_lsp_check_failure(&error, back),
        }
        true
    }

    pub(super) fn finish_lsp_check(
        &mut self,
        id: &str,
        observation: CheckObservation,
        back: Panel,
    ) {
        if back == Panel::Form {
            if let Some(draft) = &mut self.lsp.draft
                && draft.id == id
            {
                draft.last_check = Some(observation);
                draft.last_check_failure = None;
            }
            self.show_lsp_form();
        } else {
            let recorded = self.lsp_store().record_check(id, Ok(observation));
            if !self.reload_lsp_check(id) {
                return;
            }
            match recorded {
                Ok(true) => self.show_lsp_detail(),
                Ok(false) => {
                    self.show_lsp_back(Panel::List);
                    self.warn("Check completed with displayed settings, but saved settings changed or the server was removed · result not recorded");
                    return;
                }
                Err(error) => {
                    self.show_lsp_detail();
                    self.warn(&error);
                    return;
                }
            }
        }
        self.state.notify(Feedback::result(
            Kind::Notice,
            "Server initialized and stopped · check complete",
        ));
    }

    pub(super) fn finish_lsp_check_failure(&mut self, error: &str, back: Panel) {
        if back == Panel::Form {
            if let Some(draft) = &mut self.lsp.draft {
                let fingerprint = draft.analysis_fingerprint();
                draft.last_check = None;
                draft.last_check_failure = Some(CheckFailure {
                    detail: error.into(),
                    fingerprint,
                });
            }
            self.show_lsp_form();
            self.warn(error);
            return;
        }
        let Some(profile) = self.selected_lsp().cloned() else {
            return;
        };
        let failure = CheckFailure {
            detail: error.into(),
            fingerprint: profile.analysis_fingerprint(),
        };
        let recorded = self.lsp_store().record_check(&profile.id, Err(failure));
        if !self.reload_lsp_check(&profile.id) {
            return;
        }
        match recorded {
            Ok(true) => {
                self.show_lsp_detail();
                self.warn(error);
            }
            Ok(false) => {
                self.show_lsp_list();
                self.warn("Check failed with displayed settings, but saved settings changed or the server was removed · failure not recorded");
            }
            Err(storage_error) => {
                self.show_lsp_detail();
                self.warn(&storage_error);
            }
        }
    }

    fn reload_lsp_check(&mut self, id: &str) -> bool {
        match self.lsp_store().load() {
            Ok(profiles) => {
                for profile in &profiles {
                    for value in profile.environment.values() {
                        self.archive.redactor.add_secret(value.clone());
                    }
                }
                self.lsp.profiles = profiles;
                if !self.lsp.profiles.iter().any(|profile| profile.id == id) {
                    self.lsp.selected = None;
                }
                true
            }
            Err(error) => {
                self.show_lsp_back(Panel::List);
                self.warn(&error);
                false
            }
        }
    }
}
