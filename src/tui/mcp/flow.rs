use super::{App, AuthMethod, Connection, Field, Job, Panel, Purpose, ResultValue, Selector};
use crate::mcp::auth::Update;
use crate::tui::{Feedback, Kind};
use std::sync::mpsc;

impl App {
    fn mcp_change(&mut self, message: &str) {
        if self.local_command.as_deref() == Some("/settings") {
            self.local_finish_change(Kind::Notice, message, vec![]);
        } else {
            self.state.notify(
                Feedback::result(Kind::Notice, self.archive.redactor.text(message)).change(),
            );
        }
    }

    fn mcp_notice(&mut self, message: &str) {
        self.state.notify(Feedback::result(
            Kind::Notice,
            self.archive.redactor.text(message),
        ));
    }

    fn mcp_error(&mut self, message: &str) {
        self.state.notify(Feedback::result(
            Kind::Error,
            self.archive.redactor.text(message),
        ));
    }

    pub(in crate::tui) fn close_mcp_panel(&mut self) -> bool {
        let panel = match self.state.selector.as_ref().map(|menu| &menu.purpose) {
            Some(Purpose::Mcp(panel)) => *panel,
            _ => return false,
        };
        self.mcp.job.take();
        self.mcp.updates = None;
        self.mcp.inventory_updates = None;
        match panel {
            Panel::List => {
                if self.mcp.from_inspection {
                    self.mcp.settings = false;
                    self.mcp.from_inspection = false;
                    self.show_mcp_list();
                    return true;
                }
                let settings = self.mcp.settings;
                self.mcp.clear();
                self.settings_parent = None;
                self.state.selector = settings.then(|| self.settings_menu());
                if !settings {
                    self.state.status = "Ready".into();
                }
            }
            Panel::Add => self.show_mcp_list(),
            Panel::Detail => {
                if self.mcp.from_inspection {
                    self.mcp.settings = false;
                    self.mcp.from_inspection = false;
                }
                self.show_mcp_list();
            }
            Panel::Form => {
                self.mcp.draft = None;
                if self.mcp.selected.is_some() {
                    self.show_mcp_detail();
                } else {
                    self.show_mcp_add();
                }
            }
            Panel::Field(_) | Panel::Methods | Panel::Advanced => self.show_mcp_form(),
            Panel::Loading => {
                if self.mcp.list_pending {
                    let settings = self.mcp.settings;
                    self.mcp.clear();
                    self.settings_parent = None;
                    self.state.selector = settings.then(|| self.settings_menu());
                } else if self.mcp.probe_inspection {
                    self.mcp.probe_inspection = false;
                    self.show_mcp_list();
                } else if self.mcp.draft.is_some() {
                    self.show_mcp_form();
                } else if self.mcp.selected.is_some() {
                    self.show_mcp_detail();
                } else {
                    self.show_mcp_list();
                }
            }
            Panel::ConfirmRemove => self.show_mcp_detail(),
            Panel::Tools => self.show_mcp_list(),
        }
        true
    }

    pub(in crate::tui) fn mcp_empty_enter(&mut self) -> bool {
        if self
            .state
            .selector
            .as_ref()
            .is_some_and(|menu| matches!(menu.purpose, Purpose::Mcp(Panel::List)))
            && !self.mcp.settings
            && self.mcp.connections.is_empty()
        {
            self.mcp.settings = true;
            self.mcp.from_inspection = true;
            self.show_mcp_list();
            return true;
        }
        false
    }

    pub(super) fn mcp_loading(&mut self, message: &str) {
        let mut menu = Selector::mcp(Panel::Loading, "MCP", Some("Settings"), "", vec![]);
        menu.message = message.into();
        self.state.selector = Some(menu);
    }

    pub(super) fn start_mcp_methods(&mut self) {
        let Some(draft) = self.mcp.draft.clone() else {
            return;
        };
        if draft.endpoint.trim().is_empty() {
            self.show_mcp_field(Field::Endpoint);
            return;
        }
        if let Some(methods) = self
            .mcp
            .method_cache
            .get(&super::Ui::methods_key(&draft))
            .cloned()
        {
            self.mcp.methods = methods;
            self.present_mcp_methods();
            return;
        }
        self.mcp.inventory_updates = None;
        self.mcp.job = Some(Job::methods(draft));
        self.mcp_loading("Checking server sign-in options…");
    }

    pub(super) fn present_mcp_methods(&mut self) {
        if self.mcp.methods == [AuthMethod::None] {
            if let Some(draft) = &mut self.mcp.draft {
                draft.method = AuthMethod::None;
            }
            self.show_mcp_form();
            self.mcp_notice("This server does not require sign-in");
            return;
        }
        self.mcp
            .methods
            .retain(|method| *method != AuthMethod::None);
        self.show_mcp_methods();
    }

    pub(super) fn start_mcp_probe(&mut self, connection: Connection) {
        self.mcp.inventory_updates = None;
        self.mcp.job = Some(Job::probe(self.mcp_home(), connection.id));
        self.mcp_loading("Checking connection and loading tools…");
    }

    pub(super) fn start_mcp_save(&mut self) {
        let Some(draft) = self.mcp.draft.clone() else {
            return;
        };
        if draft.name.trim().is_empty() {
            self.show_mcp_field(Field::Name);
            return;
        }
        if draft.endpoint.trim().is_empty() {
            self.show_mcp_field(Field::Endpoint);
            return;
        }
        if draft.method == AuthMethod::None && self.mcp.methods != [AuthMethod::None] {
            self.start_mcp_methods();
            return;
        }
        if draft.method == AuthMethod::Token && draft.token.is_empty() {
            self.show_mcp_field(Field::Token);
            return;
        }
        self.mcp.inventory_updates = None;
        self.mcp.job = Some(Job::save(self.mcp_home(), draft));
        self.mcp_loading("Checking connection and loading tools…");
    }

    pub(super) fn start_mcp_sign_in(&mut self) {
        let Some(draft) = self.mcp.draft.clone() else {
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.mcp.updates = Some(receiver);
        self.mcp.browser_url = None;
        self.mcp.device_code = None;
        self.mcp.inventory_updates = None;
        self.mcp.job = Some(Job::sign_in(draft, sender));
        self.mcp_loading("Starting sign-in…");
    }

    pub(super) fn start_mcp_remove(&mut self) {
        let Some(id) = self.mcp.selected.clone() else {
            return;
        };
        self.mcp.inventory_updates = None;
        self.mcp.job = Some(Job::remove(self.mcp_home(), id));
        self.mcp_loading("Removing connection and sign-in…");
    }

    pub(super) fn show_mcp_tools(&mut self) {
        let Some(connection) = self.selected_mcp() else {
            return;
        };
        let Some(Ok(catalog)) = self.mcp.health.get(&connection.id) else {
            self.show_mcp_detail();
            return;
        };
        let title = connection.name.clone();
        let rows = catalog
            .tools
            .iter()
            .map(|tool| {
                (
                    tool.title.clone().unwrap_or_else(|| tool.name.clone()),
                    tool.description.clone(),
                )
            })
            .collect();
        self.state.selector = Some(Selector::mcp(
            Panel::Tools,
            &title,
            Some("MCP"),
            "tools",
            rows,
        ));
    }

    pub(super) fn show_mcp_tool(&mut self) {
        let Some(connection) = self.selected_mcp() else {
            return;
        };
        let Some(Ok(catalog)) = self.mcp.health.get(&connection.id) else {
            return;
        };
        let Some(tool) = catalog.tools.get(self.mcp.tool) else {
            return;
        };
        let description = tool.description.trim();
        let input = tool.input_schema.encode();
        let mut rows = vec![("Operation".into(), tool.name.clone())];
        if !description.is_empty() {
            rows.push(("Description".into(), description.into()));
        }
        rows.push(("Inputs".into(), input));
        if let Some(hints) = &tool.annotations {
            rows.push(("Server claims".into(), hints.encode()));
        }
        self.state.information = Some(
            super::super::information::Information::new(
                tool.title.clone().unwrap_or_else(|| tool.name.clone()),
                rows,
            )
            .context(&connection.name),
        );
    }

    pub(in crate::tui) fn poll_mcp(&mut self) -> bool {
        if !self.mcp.open {
            return false;
        }
        let mut dirty = false;
        if let Some(updates) = &self.mcp.updates {
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
        }
        let inventory = self
            .mcp
            .inventory_updates
            .as_ref()
            .map(|updates| updates.try_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        if !inventory.is_empty() {
            for (id, connection, result) in inventory {
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
            if self
                .state
                .selector
                .as_ref()
                .is_some_and(|menu| matches!(menu.purpose, Purpose::Mcp(Panel::List)))
            {
                self.show_mcp_list();
            }
            dirty = true;
        }
        if !self.mcp.job.as_ref().is_some_and(Job::finished) {
            return dirty;
        }
        let result = self.mcp.job.take().expect("finished MCP job").finish();
        self.mcp.updates = None;
        let Ok(result) = result else {
            self.mcp_error("MCP worker failed");
            self.show_mcp_form();
            return true;
        };
        match result {
            ResultValue::List(result) => {
                self.mcp.list_pending = false;
                match result {
                    Ok(connections) => {
                        self.mcp.connections = connections;
                        self.mcp.health.clear();
                        self.show_mcp_list();
                        let (sender, receiver) = mpsc::channel();
                        self.mcp.inventory_updates = Some(receiver);
                        self.mcp.job = Some(Job::inventory(
                            self.mcp_home(),
                            self.mcp.connections.clone(),
                            sender,
                        ));
                    }
                    Err(error) => {
                        let settings = self.mcp.settings;
                        self.mcp.clear();
                        self.settings_parent = None;
                        self.state.selector = settings.then(|| self.settings_menu());
                        self.mcp_error(&error.message);
                    }
                }
            }
            ResultValue::Inventory => {
                let tail = self
                    .mcp
                    .inventory_updates
                    .take()
                    .map(|updates| updates.try_iter().collect::<Vec<_>>())
                    .unwrap_or_default();
                if !tail.is_empty() {
                    for (id, connection, result) in tail {
                        if let Some(connection) = connection {
                            connection.protect(&self.archive.redactor);
                        }
                        self.mcp.health.insert(id, result);
                    }
                    if self
                        .state
                        .selector
                        .as_ref()
                        .is_some_and(|menu| matches!(menu.purpose, Purpose::Mcp(Panel::List)))
                    {
                        self.show_mcp_list();
                    }
                }
            }
            ResultValue::Methods(result) => match result {
                Ok(methods) => {
                    if let Some(draft) = &self.mcp.draft {
                        self.mcp
                            .method_cache
                            .insert(super::Ui::methods_key(draft), methods.clone());
                    }
                    self.mcp.methods = methods;
                    self.present_mcp_methods();
                }
                Err(error) => {
                    self.show_mcp_form();
                    self.mcp_error(&error.message);
                }
            },
            ResultValue::Probe(id, connection, result) => {
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
                match result {
                    Ok(catalog) => {
                        self.mcp.health.insert(id.clone(), Ok(catalog));
                        if self.mcp.probe_inspection {
                            self.mcp.probe_inspection = false;
                            self.show_mcp_tools();
                        } else {
                            self.show_mcp_detail();
                            self.mcp_notice("Connected · tools loaded");
                        }
                    }
                    Err(error) => {
                        self.mcp.health.insert(id, Err(error.kind));
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
            ResultValue::Saved(result) => match result {
                Ok((saved, catalog)) => {
                    saved.protect(&self.archive.redactor);
                    let home = self.mcp_home();
                    if let Some(agent) = &mut self.agent
                        && let Err(error) = agent.configure_mcp(&home)
                    {
                        self.mcp_error(&error);
                    }
                    self.mcp.health.insert(saved.id.clone(), Ok(catalog));
                    self.mcp.selected = Some(saved.id.clone());
                    self.mcp.connections.retain(|c| c.id != saved.id);
                    self.mcp.connections.push(saved);
                    self.mcp.draft = None;
                    self.show_mcp_detail();
                    self.mcp_change("Connection saved and available");
                }
                Err(error) => {
                    self.show_mcp_form();
                    self.mcp_error(&error.message);
                }
            },
            ResultValue::SignedIn(result) => match result {
                Ok(connection) => {
                    self.mcp.draft = Some(connection);
                    if self.mcp.sign_in_saved {
                        self.start_mcp_save();
                    } else {
                        self.show_mcp_form();
                        self.mcp_notice("Signed in · save the connection");
                    }
                }
                Err(error) => {
                    self.show_mcp_form();
                    self.mcp_error(&error.message);
                }
            },
            ResultValue::Removed(id, result) => match result {
                Ok(()) => {
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
                    if let Some(c) = self.mcp.connections.iter_mut().find(|c| c.id == id) {
                        c.enabled = enabled;
                    }
                    self.mcp.health.remove(&id);
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
        true
    }
}
