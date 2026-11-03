//! Composer fields of a connection draft, validated as the backend will.
use super::{App, AuthMethod, Field, Panel, Purpose, Selector, Ui};

impl App {
    pub(super) fn show_mcp_field(&mut self, field: Field) {
        let Some(draft) = self.mcp.connection() else {
            self.show_mcp_detail();
            return;
        };
        let (title, value) = match field {
            Field::Name => ("Name", draft.name.clone()),
            Field::Endpoint => ("Endpoint", draft.endpoint.clone()),
            Field::Token => ("Token", draft.token.clone()),
            Field::Timeout => ("Timeout", draft.timeout_secs.to_string()),
            Field::Header => ("Extra header", String::new()),
            Field::ClientId => (
                "OAuth client ID",
                draft.oauth_client_id.clone().unwrap_or_default(),
            ),
        };
        let mut menu = Selector::mcp(Panel::Field(field), title, Some("Connection"), "", vec![]);
        menu.editor.replace(value);
        self.state.selector = Some(menu);
    }

    /// Why a name cannot be saved: the backend's rules and a distinct name.
    pub(super) fn mcp_name_problem(&self, name: &str) -> Option<&'static str> {
        let id = self
            .mcp
            .connection()
            .map(|c| c.id.as_str())
            .unwrap_or_default();
        if name.trim().is_empty() {
            Some("Enter a connection name")
        } else if name.trim() != name || name.len() > 128 || name.chars().any(char::is_control) {
            Some("Use 1–128 printable characters for the name")
        } else if self
            .mcp
            .connections
            .iter()
            .any(|c| c.id != id && c.name.eq_ignore_ascii_case(name))
        {
            Some("A connection with this name already exists")
        } else {
            None
        }
    }

    pub(in crate::tui) fn save_mcp_field(&mut self) {
        let Some(menu) = &self.state.selector else {
            return;
        };
        let Purpose::Mcp(Panel::Field(field)) = menu.purpose else {
            return;
        };
        let value = menu.editor.text.trim().to_owned();
        if field == Field::Name
            && let Some(problem) = self.mcp_name_problem(&value)
        {
            self.warn(problem);
            return;
        }
        if field == Field::Endpoint
            && let Err(error) = crate::mcp::config::validate_url(&value)
        {
            self.warn(&error.message);
            return;
        }
        let Some(draft) = &mut self.mcp.draft else {
            return;
        };
        let connection = &mut draft.connection;
        match field {
            Field::Name => connection.name = value,
            Field::Endpoint => {
                if connection.endpoint != value {
                    // A different server: its identity, credentials and methods are new.
                    connection.endpoint = value;
                    connection.method = AuthMethod::None;
                    connection.token.clear();
                    connection.oauth = None;
                    connection.oauth_client_id = None;
                    connection.pending_scope = None;
                    connection.headers.clear();
                    draft.methods = self
                        .mcp
                        .method_cache
                        .get(&Ui::methods_key(connection))
                        .cloned()
                        .unwrap_or_default();
                }
            }
            Field::Token => {
                connection.method = AuthMethod::Token;
                connection.token = value;
                connection.oauth = None;
            }
            Field::Timeout => match value.parse::<u64>() {
                Ok(seconds @ 1..=600) => connection.timeout_secs = seconds,
                _ => {
                    self.warn("Timeout must be 1–600 seconds");
                    return;
                }
            },
            Field::Header => {
                let Some((name, value)) = value.split_once(':') else {
                    self.warn("Use Header-Name: value");
                    return;
                };
                if name.trim().is_empty() {
                    self.warn("Enter a header name");
                    return;
                }
                connection
                    .headers
                    .retain(|(key, _)| !key.eq_ignore_ascii_case(name.trim()));
                if !value.trim().is_empty() {
                    connection
                        .headers
                        .push((name.trim().into(), value.trim().into()));
                }
                draft.methods.clear();
                self.mcp.method_cache.clear();
            }
            Field::ClientId => {
                connection.oauth_client_id = (!value.is_empty()).then_some(value);
                draft.methods = self
                    .mcp
                    .method_cache
                    .get(&Ui::methods_key(connection))
                    .cloned()
                    .unwrap_or_default();
            }
        }
        connection.protect(&self.archive.redactor);
        let sign_in = draft.sign_in;
        match field {
            Field::Timeout | Field::Header | Field::ClientId => self.show_mcp_advanced(),
            // A saved connection's Sign in action completes by saving.
            Field::Token if sign_in => self.start_mcp_save(),
            Field::Name | Field::Endpoint | Field::Token => self.show_mcp_form(),
        }
    }
}
