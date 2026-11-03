//! Navigation between B14 panels and the jobs they start.
use super::{App, AuthMethod, Connection, Field, Health, Job, Panel, Purpose, Selector};
use crate::tui::{Feedback, Kind, selector::Setting};
use std::sync::mpsc;

impl App {
    pub(super) fn mcp_change(&mut self, message: &str) {
        if self.local_command.as_deref() == Some("/settings") {
            self.local_finish_change(Kind::Notice, message, vec![]);
        } else {
            self.state.notify(
                Feedback::result(Kind::Notice, self.archive.redactor.text(message)).change(),
            );
        }
    }

    pub(super) fn mcp_notice(&mut self, message: &str) {
        self.state.notify(Feedback::result(
            Kind::Notice,
            self.archive.redactor.text(message),
        ));
    }

    pub(super) fn mcp_error(&mut self, message: &str) {
        self.state.notify(Feedback::result(
            Kind::Error,
            self.archive.redactor.text(message),
        ));
    }

    /// Leave MCP entirely: back to Settings on its MCP row, or to the chat.
    pub(super) fn leave_mcp(&mut self) {
        let settings = self.mcp.settings;
        self.mcp.clear();
        if settings {
            self.settings_parent = Some(Setting::Mcp);
            self.return_to_settings();
        } else {
            self.settings_parent = None;
            self.state.selector = None;
            self.state.status = "Ready".into();
        }
    }

    /// Esc walks back exactly one panel level.
    pub(in crate::tui) fn close_mcp_panel(&mut self) -> bool {
        let panel = match self.state.selector.as_ref().map(|menu| &menu.purpose) {
            Some(Purpose::Mcp(panel)) => *panel,
            _ => return false,
        };
        self.mcp.job.take();
        self.mcp.updates = None;
        match panel {
            Panel::List if self.mcp.from_inspection => self.return_to_inspection(),
            Panel::List => self.leave_mcp(),
            Panel::Add | Panel::Tools => self.show_mcp_list(),
            Panel::Detail(_) if self.mcp.from_inspection => self.return_to_inspection(),
            Panel::Detail(_) => self.show_mcp_list(),
            Panel::ConfirmRemove => self.show_mcp_detail(),
            Panel::Loading => self.return_from_loading(),
            Panel::Form | Panel::Methods | Panel::Advanced | Panel::Field(_) => {
                let parent = self.mcp_draft_parent(panel);
                if matches!(parent, Panel::Add | Panel::Detail(_)) {
                    self.mcp.draft = None;
                }
                self.show_mcp_panel(parent);
            }
        }
        true
    }

    /// The panel one level above a draft panel.
    fn mcp_draft_parent(&self, panel: Panel) -> Panel {
        let Some(draft) = &self.mcp.draft else {
            return Panel::List;
        };
        let start = if draft.original.is_some() {
            Panel::Detail(Health::Unchecked)
        } else {
            Panel::Add
        };
        match panel {
            Panel::Methods if draft.sign_in => start,
            Panel::Form => start,
            Panel::Field(Field::Endpoint)
                if draft.original.is_none() && draft.connection.endpoint.is_empty() =>
            {
                Panel::Add
            }
            Panel::Field(Field::Token) if !draft.methods.is_empty() => Panel::Methods,
            Panel::Field(Field::Timeout | Field::Header | Field::ClientId) => Panel::Advanced,
            _ => Panel::Form,
        }
    }

    fn return_to_inspection(&mut self) {
        self.mcp.settings = false;
        self.mcp.from_inspection = false;
        self.show_mcp_list();
    }

    /// Back to the panel that started the cancelled or failed work.
    pub(super) fn return_from_loading(&mut self) {
        if self.mcp.list_pending {
            self.leave_mcp();
            return;
        }
        self.mcp.probe_inspection = false;
        let back = self.mcp.back.take().unwrap_or(Panel::List);
        self.show_mcp_panel(back);
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

    pub(super) fn mcp_loading(&mut self, message: &str, back: Panel) {
        self.mcp.back = Some(back);
        let parent = self.mcp.settings.then_some("Settings");
        let mut menu = Selector::mcp(Panel::Loading, "MCP", parent, "", vec![]);
        menu.message = message.into();
        self.state.selector = Some(menu);
    }

    /// Where a draft's sign-in steps return: its form, or the connection
    /// whose Sign in action started them.
    fn mcp_sign_in_origin(&self) -> Panel {
        if self.mcp.draft.as_ref().is_some_and(|draft| draft.sign_in) {
            Panel::Detail(Health::Unchecked)
        } else {
            Panel::Form
        }
    }

    pub(super) fn start_mcp_methods(&mut self) {
        let Some(draft) = self.mcp.draft.as_mut() else {
            return;
        };
        let connection = draft.connection.clone();
        if connection.endpoint.trim().is_empty() {
            self.show_mcp_field(Field::Endpoint);
            return;
        }
        if let Err(error) = crate::mcp::config::validate_url(&connection.endpoint) {
            self.warn(&error.message);
            self.show_mcp_field(Field::Endpoint);
            return;
        }
        if let Some(methods) = self
            .mcp
            .method_cache
            .get(&super::Ui::methods_key(&connection))
        {
            draft.methods = methods.clone();
            self.present_mcp_methods();
            return;
        }
        self.mcp.job = Some(Job::methods(connection));
        let origin = self.mcp_sign_in_origin();
        self.mcp_loading("Checking server sign-in options…", origin);
    }

    pub(super) fn start_mcp_probe(&mut self, connection: Connection) {
        self.mcp.job = Some(Job::probe(self.mcp_home(), connection.id));
        let back = if self.mcp.probe_inspection {
            Panel::List
        } else {
            Panel::Detail(Health::Unchecked)
        };
        self.mcp_loading("Checking connection and loading tools…", back);
    }

    pub(super) fn start_mcp_save(&mut self) {
        let Some(draft) = &self.mcp.draft else {
            return;
        };
        let connection = draft.connection.clone();
        let sign_in_guard = draft.sign_in_guard.clone();
        let public = draft.methods == [AuthMethod::None];
        // Tokens loaded with the saved connection, not from a new sign-in.
        let kept = sign_in_guard.is_none()
            && draft.original.as_ref().is_some_and(|original| {
                original.oauth.is_some() && original.oauth == connection.oauth
            });
        let back = if draft.sign_in {
            Panel::Methods
        } else {
            Panel::Form
        };
        if let Some(problem) = self.mcp_name_problem(&connection.name) {
            self.warn(problem);
            self.show_mcp_field(Field::Name);
            return;
        }
        if let Err(error) = crate::mcp::config::validate_url(&connection.endpoint) {
            self.warn(&error.message);
            self.show_mcp_field(Field::Endpoint);
            return;
        }
        let oauth = matches!(
            connection.method,
            AuthMethod::Browser | AuthMethod::GitHubDevice
        );
        if (connection.method == AuthMethod::None && !public)
            || (oauth && connection.oauth.is_none())
        {
            self.start_mcp_methods();
            return;
        }
        if connection.method == AuthMethod::Token && connection.token.is_empty() {
            self.show_mcp_field(Field::Token);
            return;
        }
        if let Err(error) = crate::mcp::config::validate(&connection) {
            self.show_mcp_panel(back);
            self.warn(&error.message);
            return;
        }
        self.mcp.job = Some(Job::save(self.mcp_home(), connection, kept, sign_in_guard));
        self.mcp_loading("Checking connection and loading tools…", back);
    }

    pub(super) fn start_mcp_sign_in(&mut self) {
        let Some(draft) = &mut self.mcp.draft else {
            return;
        };
        draft.sign_in_guard = None;
        let connection = draft.connection.clone();
        let original = draft.original.clone();
        let (sender, receiver) = mpsc::channel();
        self.mcp.updates = Some(receiver);
        self.mcp.browser_url = None;
        self.mcp.device_code = None;
        self.mcp.job = Some(Job::sign_in(self.mcp_home(), connection, original, sender));
        self.mcp_loading("Starting sign-in…", Panel::Methods);
    }

    pub(super) fn start_mcp_remove(&mut self) {
        let Some(id) = self.mcp.selected.clone() else {
            return;
        };
        self.mcp.job = Some(Job::remove(self.mcp_home(), id));
        self.mcp_loading("Removing connection and sign-in…", Panel::ConfirmRemove);
    }

    /// Check connections in the background, without blocking navigation.
    pub(super) fn check_mcp(&mut self, ids: Vec<String>) {
        let home = self.mcp_home();
        self.mcp.checks.start(home, ids);
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
            crate::tui::information::Information::new(
                tool.title.clone().unwrap_or_else(|| tool.name.clone()),
                rows,
            )
            .context(&connection.name),
        );
    }
}
