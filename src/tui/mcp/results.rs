//! Worker results applied to the panels: sign-in progress, background checks
//! and the outcome of the foreground job.
use super::{App, AuthMethod, Catalog, Connection, ErrorKind, Field, Panel, Purpose, ResultValue};
use crate::mcp::auth::Update;

impl App {
    pub(in crate::tui) fn poll_mcp(&mut self) -> bool {
        if !self.mcp.open {
            return false;
        }
        let mut dirty = self.poll_mcp_sign_in();
        let checked = self.mcp.checks.take();
        if !checked.is_empty() {
            for (id, connection, result) in checked {
                self.absorb_mcp(id, connection, result);
            }
            self.refresh_mcp_status();
            dirty = true;
        }
        if !self.mcp.job.as_ref().is_some_and(super::Job::finished) {
            return dirty;
        }
        let result = self.mcp.job.take().expect("finished MCP job").finish();
        self.mcp.updates = None;
        match result {
            Ok(result) => self.apply_mcp(result),
            Err(error) => {
                self.return_from_loading();
                self.mcp_error(&error);
            }
        }
        true
    }

    fn poll_mcp_sign_in(&mut self) -> bool {
        let Some(updates) = &self.mcp.updates else {
            return false;
        };
        let mut dirty = false;
        while let Ok(update) = updates.try_recv() {
            if let Some(menu) = &mut self.state.selector {
                menu.message = match update {
                    Update::Browser(url) => {
                        self.mcp.browser_url = Some(url.clone());
                        format!("Open browser · {url}")
                    }
                    Update::DeviceCode { url, code } => {
                        self.mcp.browser_url = Some(url.clone());
                        self.mcp.device_code = Some(code.clone());
                        format!("Open {url} · enter code {code}")
                    }
                    Update::Waiting => match (&self.mcp.browser_url, &self.mcp.device_code) {
                        (Some(url), Some(code)) => {
                            format!("Waiting · open {url} · enter code {code}")
                        }
                        (Some(url), None) => format!("Waiting for sign-in · {url}"),
                        _ => "Waiting for sign-in…".into(),
                    },
                };
            }
            dirty = true;
        }
        dirty
    }

    /// Record a connection's availability with the current record it was
    /// checked against, which may carry refreshed OAuth tokens.
    fn absorb_mcp(
        &mut self,
        id: String,
        connection: Option<Connection>,
        result: Result<Catalog, ErrorKind>,
    ) {
        if let Some(connection) = connection {
            connection.protect(&self.archive.redactor);
            if let Some(saved) = self
                .mcp
                .connections
                .iter_mut()
                .find(|saved| saved.id == connection.id)
            {
                *saved = connection;
            }
        }
        self.mcp.health.insert(id, result);
    }

    /// Repaint a panel that shows availability, keeping its focus.
    fn refresh_mcp_status(&mut self) {
        match self.state.selector.as_ref().map(|menu| &menu.purpose) {
            Some(Purpose::Mcp(Panel::List)) => self.show_mcp_list(),
            Some(Purpose::Mcp(Panel::Detail(_))) => {
                let selected = self.state.selector.as_ref().map_or(0, |menu| menu.selected);
                self.show_mcp_detail();
                if let Some(menu) = &mut self.state.selector {
                    menu.selected = selected.min(menu.filtered.len().saturating_sub(1));
                }
            }
            _ => {}
        }
    }

    fn apply_mcp(&mut self, result: ResultValue) {
        match result {
            ResultValue::List(Ok(connections)) => {
                self.mcp.list_pending = false;
                self.mcp.connections = connections;
                self.mcp.health.clear();
                let ids = self
                    .mcp
                    .connections
                    .iter()
                    .filter(|c| c.enabled)
                    .map(|c| c.id.clone())
                    .collect();
                self.check_mcp(ids);
                self.show_mcp_list();
            }
            ResultValue::List(Err(error)) => {
                self.leave_mcp();
                self.mcp_error(&error.message);
            }
            ResultValue::Inventory => {}
            ResultValue::Methods(result) => match result {
                Ok(methods) => {
                    if let Some(draft) = &mut self.mcp.draft {
                        let key = super::Ui::methods_key(&draft.connection);
                        self.mcp.method_cache.insert(key, methods.clone());
                        draft.methods = methods;
                    }
                    self.present_mcp_methods();
                }
                Err(error) => {
                    self.return_from_loading();
                    self.mcp_error(&error.message);
                }
            },
            ResultValue::Probe(id, connection, result) => {
                self.mcp.checks.forget(&id);
                let kind = result.as_ref().cloned().map_err(|e| e.kind);
                self.absorb_mcp(id, connection, kind);
                match result {
                    Ok(_) if self.mcp.probe_inspection => {
                        self.mcp.probe_inspection = false;
                        self.show_mcp_tools();
                    }
                    Ok(_) => {
                        self.show_mcp_detail();
                        self.mcp_notice("Connected · tools loaded");
                    }
                    Err(error) => {
                        if self.mcp.probe_inspection {
                            self.mcp.probe_inspection = false;
                            self.mcp.settings = true;
                            self.mcp.from_inspection = true;
                        }
                        self.show_mcp_detail();
                        self.mcp_error(&error.message);
                    }
                }
            }
            ResultValue::Saved(result) => self.apply_mcp_saved(result),
            ResultValue::SignedIn(result) => match result {
                Ok((connection, guard)) => {
                    connection.protect(&self.archive.redactor);
                    let Some(draft) = &mut self.mcp.draft else {
                        return;
                    };
                    draft.connection = connection;
                    draft.sign_in_guard = guard;
                    if draft.sign_in {
                        self.start_mcp_save();
                    } else {
                        self.show_mcp_form();
                        self.mcp_notice("Signed in · save the connection");
                    }
                }
                Err(error) => {
                    // The form names the unfinished OAuth sign-in.
                    if self.mcp.draft.as_ref().is_some_and(|draft| draft.sign_in) {
                        self.show_mcp_methods();
                    } else {
                        self.show_mcp_form();
                    }
                    self.mcp_error(&error.message);
                }
            },
            ResultValue::Removed(id, result) => match result {
                Ok(()) => {
                    self.mcp.checks.forget(&id);
                    self.mcp.connections.retain(|c| c.id != id);
                    self.mcp.health.remove(&id);
                    self.mcp.selected = None;
                    self.show_mcp_list();
                    self.mcp_change("Connection removed");
                }
                Err(error) => {
                    self.show_mcp_detail();
                    self.mcp_error(&error.message);
                }
            },
            ResultValue::Enabled(id, enabled, result) => match result {
                Ok(()) => {
                    self.mcp.checks.forget(&id);
                    if let Some(c) = self.mcp.connections.iter_mut().find(|c| c.id == id) {
                        c.enabled = enabled;
                    }
                    self.mcp.health.remove(&id);
                    if enabled {
                        self.check_mcp(vec![id]);
                    }
                    self.show_mcp_detail();
                    self.mcp_change(if enabled {
                        "Connection resumed globally"
                    } else {
                        "Connection suspended globally"
                    });
                }
                Err(error) => {
                    self.show_mcp_detail();
                    self.mcp_error(&error.message);
                }
            },
        }
    }

    fn apply_mcp_saved(&mut self, result: Result<(Connection, Catalog), crate::mcp::Error>) {
        match result {
            Ok((saved, catalog)) => {
                saved.protect(&self.archive.redactor);
                let home = self.mcp_home();
                if let Some(agent) = &mut self.agent
                    && let Err(error) = agent.configure_mcp(&home)
                {
                    self.mcp_error(&error);
                }
                self.mcp.checks.forget(&saved.id);
                self.mcp.health.insert(saved.id.clone(), Ok(catalog));
                self.mcp.selected = Some(saved.id.clone());
                self.mcp.connections.retain(|c| c.id != saved.id);
                self.mcp.connections.push(saved);
                self.mcp.draft = None;
                self.show_mcp_detail();
                self.mcp_change("Connection saved and available");
            }
            Err(error) => {
                // Failure returns to the step to correct.
                let token = self.mcp.connection().map(|c| c.method) == Some(AuthMethod::Token);
                match self.mcp.draft.as_ref().map(|draft| draft.sign_in) {
                    Some(true) if token => self.show_mcp_field(Field::Token),
                    Some(true) => self.show_mcp_methods(),
                    _ => self.show_mcp_form(),
                }
                self.mcp_error(&error.message);
            }
        }
    }
}
