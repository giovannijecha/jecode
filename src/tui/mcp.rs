//! Global MCP configuration and read-only inspection in the B5 selector.
mod flow;
#[cfg(test)]
mod integration_tests;
mod job;
#[cfg(test)]
mod tests;

use super::{
    App,
    selector::{Purpose, Selector},
};
use crate::mcp::{AuthMethod, Catalog, Connection, ErrorKind};
use job::{InventoryResult, Job, ResultValue};
use std::{collections::HashMap, path::PathBuf, sync::mpsc::Receiver};

pub(super) const PRESETS: [(&str, &str, &str); 4] = [
    (
        "GitHub",
        "https://api.githubcopilot.com/mcp/",
        "api.githubcopilot.com",
    ),
    ("Supabase", "https://mcp.supabase.com/mcp", "supabase.com"),
    (
        "Cloudflare",
        "https://mcp.cloudflare.com/mcp",
        "mcp.cloudflare.com",
    ),
    (
        "Vercel",
        "https://mcp.vercel.com",
        "Client approval required",
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Field {
    Name,
    Endpoint,
    Token,
    Timeout,
    Header,
    ClientId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Panel {
    List,
    Add,
    Detail,
    Form,
    Field(Field),
    Methods,
    Advanced,
    ConfirmRemove,
    Tools,
    Loading,
}

#[derive(Default)]
pub(super) struct Ui {
    open: bool,
    settings: bool,
    from_inspection: bool,
    connections: Vec<Connection>,
    health: HashMap<String, Result<Catalog, ErrorKind>>,
    selected: Option<String>,
    draft: Option<Connection>,
    methods: Vec<AuthMethod>,
    method_cache: HashMap<String, Vec<AuthMethod>>,
    tool: usize,
    job: Option<Job>,
    updates: Option<Receiver<crate::mcp::auth::Update>>,
    inventory_updates: Option<Receiver<InventoryResult>>,
    browser_url: Option<String>,
    device_code: Option<String>,
    sign_in_saved: bool,
    probe_inspection: bool,
    list_pending: bool,
}

impl Ui {
    fn clear(&mut self) {
        let cache = std::mem::take(&mut self.method_cache);
        *self = Self::default();
        self.method_cache = cache;
    }

    fn methods_key(connection: &Connection) -> String {
        format!(
            "{}\0{}\0{}",
            connection.id,
            connection.endpoint,
            connection.oauth_client_id.as_deref().unwrap_or("")
        )
    }
}

impl App {
    fn mcp_home(&self) -> PathBuf {
        self.config
            .store
            .path()
            .parent()
            .expect("configuration directory")
            .to_path_buf()
    }

    pub(super) fn open_mcp(&mut self, settings: bool) {
        self.state.information = None;
        if !self.mcp.open {
            self.mcp.clear();
        }
        self.mcp.open = true;
        self.mcp.settings = settings;
        self.mcp.from_inspection = false;
        self.mcp.list_pending = true;
        self.mcp.inventory_updates = None;
        self.mcp.job = Some(Job::list(self.mcp_home()));
        self.mcp_loading("Loading configured connections…");
    }

    fn show_mcp_list(&mut self) {
        let focus = self.state.selector.as_ref().and_then(|menu| {
            matches!(menu.purpose, Purpose::Mcp(Panel::List)).then_some(menu.selected)
        });
        let available = self
            .mcp
            .connections
            .iter()
            .filter(|c| c.enabled && self.mcp.health.get(&c.id).is_some_and(Result::is_ok))
            .count();
        let total = self.mcp.connections.len();
        let mut rows = self
            .mcp
            .connections
            .iter()
            .map(|c| {
                let status = if !c.enabled {
                    "suspended".into()
                } else {
                    match self.mcp.health.get(&c.id) {
                        Some(Ok(catalog)) => format!("{} tools", catalog.tools.len()),
                        Some(Err(ErrorKind::Authentication)) => "sign in again".into(),
                        Some(Err(_)) => "unreachable".into(),
                        None => "checking…".into(),
                    }
                };
                (c.name.clone(), status)
            })
            .collect::<Vec<_>>();
        if self.mcp.settings {
            rows.push(("Add connection".into(), String::new()));
        }
        let mut menu = Selector::mcp(
            Panel::List,
            "MCP",
            self.mcp.settings.then_some("Settings"),
            "all conversations",
            rows,
        );
        menu.message = if total == 0 && !self.mcp.settings {
            "No configured connections · open Settings › MCP".into()
        } else {
            format!("{available} available · {total} configured")
        };
        menu.selected = focus
            .unwrap_or(0)
            .min(menu.filtered.len().saturating_sub(1));
        self.state.selector = Some(menu);
    }

    fn selected_mcp(&self) -> Option<&Connection> {
        let id = self.mcp.selected.as_ref()?;
        self.mcp.connections.iter().find(|c| &c.id == id)
    }

    fn show_mcp_add(&mut self) {
        let mut rows = PRESETS
            .iter()
            .map(|(name, _, fact)| ((*name).into(), (*fact).into()))
            .collect::<Vec<_>>();
        rows.push(("Server URL".into(), "Any HTTP MCP server".into()));
        self.state.selector = Some(Selector::mcp(
            Panel::Add,
            "Add connection",
            Some("MCP"),
            "",
            rows,
        ));
    }

    fn show_mcp_detail(&mut self) {
        let Some(connection) = self.selected_mcp() else {
            self.show_mcp_list();
            return;
        };
        let name = connection.name.clone();
        let endpoint = connection.endpoint.clone();
        let enabled = connection.enabled;
        let state = if !enabled {
            "Suspended".into()
        } else {
            match self.mcp.health.get(&connection.id) {
                Some(Ok(catalog)) => format!("Connected · {} tools", catalog.tools.len()),
                Some(Err(ErrorKind::Authentication)) => "Sign-in expired".into(),
                Some(Err(_)) => "Not reachable".into(),
                None => "Not checked".into(),
            }
        };
        let rows = vec![
            ("Edit".into(), String::new()),
            ("Sign in".into(), String::new()),
            ("Reconnect".into(), String::new()),
            (
                if enabled { "Suspend" } else { "Resume" }.into(),
                String::new(),
            ),
            ("Remove".into(), String::new()),
        ];
        let mut menu = Selector::mcp(Panel::Detail, &name, Some("MCP"), "all conversations", rows);
        menu.message = format!("{state}\n{endpoint}");
        self.state.selector = Some(menu);
    }

    fn show_mcp_form(&mut self) {
        let Some(draft) = &self.mcp.draft else {
            return;
        };
        let method = match draft.method {
            AuthMethod::None => "Not needed",
            AuthMethod::Token if draft.token.is_empty() => "Token · not entered",
            AuthMethod::Token => "Token · entered, hidden",
            AuthMethod::Browser => "Browser",
            AuthMethod::GitHubDevice => "Device code",
        };
        let rows = vec![
            ("Name".into(), draft.name.clone()),
            ("Endpoint".into(), draft.endpoint.clone()),
            ("Sign-in".into(), method.into()),
            ("Advanced".into(), String::new()),
            ("Save connection".into(), String::new()),
        ];
        self.state.selector = Some(Selector::mcp(
            Panel::Form,
            "Connection",
            Some("MCP"),
            "all conversations",
            rows,
        ));
    }

    fn show_mcp_field(&mut self, field: Field) {
        let Some(draft) = &self.mcp.draft else {
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

    fn show_mcp_advanced(&mut self) {
        let Some(draft) = &self.mcp.draft else {
            return;
        };
        self.state.selector = Some(Selector::mcp(
            Panel::Advanced,
            "Advanced",
            Some("Connection"),
            "",
            vec![
                ("Timeout".into(), format!("{} seconds", draft.timeout_secs)),
                (
                    "Extra header".into(),
                    format!("{} configured, hidden", draft.headers.len()),
                ),
                (
                    "OAuth client ID".into(),
                    draft
                        .oauth_client_id
                        .as_ref()
                        .map_or("Not configured".into(), |_| "Configured".into()),
                ),
            ],
        ));
    }

    fn show_mcp_methods(&mut self) {
        let rows = self
            .mcp
            .methods
            .iter()
            .map(|method| {
                let name = match method {
                    AuthMethod::None => "Not needed",
                    AuthMethod::Token => "Token",
                    AuthMethod::Browser => "Browser",
                    AuthMethod::GitHubDevice => "Device code",
                };
                (name.into(), String::new())
            })
            .collect();
        let mut menu = Selector::mcp(
            Panel::Methods,
            "Sign-in",
            Some("Connection"),
            "server options",
            rows,
        );
        if self.mcp.methods.is_empty() {
            menu.message = "No available sign-in method · check OAuth client ID in Advanced".into();
        }
        self.state.selector = Some(menu);
    }

    pub(super) fn choose_mcp(&mut self, panel: Panel, index: usize) {
        match panel {
            Panel::List => {
                if index == self.mcp.connections.len() && self.mcp.settings {
                    self.show_mcp_add();
                    return;
                }
                let Some(c) = self.mcp.connections.get(index) else {
                    return;
                };
                self.mcp.selected = Some(c.id.clone());
                if !self.mcp.settings && c.enabled && !self.mcp.health.contains_key(&c.id) {
                    self.mcp.probe_inspection = true;
                    self.start_mcp_probe(c.clone());
                } else if self.mcp.settings
                    || !c.enabled
                    || !self.mcp.health.get(&c.id).is_some_and(Result::is_ok)
                {
                    if !self.mcp.settings {
                        self.mcp.settings = true;
                        self.mcp.from_inspection = true;
                    }
                    self.show_mcp_detail();
                } else {
                    self.show_mcp_tools();
                }
            }
            Panel::Add => {
                let (name, endpoint, _) = PRESETS.get(index).copied().unwrap_or(("", "", ""));
                self.mcp.draft = Some(Connection::draft(name, endpoint));
                if name.is_empty() {
                    self.show_mcp_field(Field::Endpoint);
                } else {
                    self.show_mcp_form();
                }
            }
            Panel::Detail => self.choose_mcp_detail(index),
            Panel::Form => match index {
                0 => self.show_mcp_field(Field::Name),
                1 => self.show_mcp_field(Field::Endpoint),
                2 => self.start_mcp_methods(),
                3 => self.show_mcp_advanced(),
                4 => self.start_mcp_save(),
                _ => {}
            },
            Panel::Advanced => match index {
                0 => self.show_mcp_field(Field::Timeout),
                1 => self.show_mcp_field(Field::Header),
                2 => self.show_mcp_field(Field::ClientId),
                _ => {}
            },
            Panel::Methods => self.choose_mcp_method(index),
            Panel::ConfirmRemove => self.start_mcp_remove(),
            Panel::Tools => {
                self.mcp.tool = index;
                self.show_mcp_tool();
            }
            Panel::Field(_) | Panel::Loading => {}
        }
    }

    fn choose_mcp_detail(&mut self, index: usize) {
        let Some(c) = self.selected_mcp().cloned() else {
            return;
        };
        match index {
            0 => {
                self.mcp.sign_in_saved = false;
                self.mcp.draft = Some(c);
                self.show_mcp_form();
            }
            1 => {
                self.mcp.sign_in_saved = true;
                self.mcp.draft = Some(c);
                self.start_mcp_methods();
            }
            2 => self.start_mcp_probe(c),
            3 => {
                self.mcp.job = Some(Job::enabled(self.mcp_home(), c.id, !c.enabled));
                self.mcp.inventory_updates = None;
                self.mcp_loading(if c.enabled {
                    "Suspending connection…"
                } else {
                    "Resuming connection…"
                });
            }
            4 => {
                self.state.selector = Some(Selector::mcp(
                    Panel::ConfirmRemove,
                    "Remove connection?",
                    Some("MCP"),
                    "sign-in removed",
                    vec![("Remove".into(), c.name)],
                ))
            }
            _ => {}
        }
    }

    fn choose_mcp_method(&mut self, index: usize) {
        let Some(method) = self.mcp.methods.get(index).copied() else {
            return;
        };
        if let Some(draft) = &mut self.mcp.draft {
            if draft.method != method {
                draft.token.clear();
                draft.oauth = None;
            }
            draft.method = method;
        }
        match method {
            AuthMethod::Token => self.show_mcp_field(Field::Token),
            AuthMethod::Browser | AuthMethod::GitHubDevice => self.start_mcp_sign_in(),
            AuthMethod::None => self.show_mcp_form(),
        }
    }

    pub(super) fn save_mcp_field(&mut self) {
        let Some(menu) = &self.state.selector else {
            return;
        };
        let Purpose::Mcp(Panel::Field(field)) = menu.purpose else {
            return;
        };
        let value = menu.editor.text.trim().to_owned();
        let Some(draft) = &mut self.mcp.draft else {
            return;
        };
        match field {
            Field::Name if value.is_empty() => {
                self.warn("Enter a connection name");
                return;
            }
            Field::Name => draft.name = value,
            Field::Endpoint => {
                if !value.starts_with("https://")
                    && !value.starts_with("http://127.0.0.1")
                    && !value.starts_with("http://localhost")
                {
                    self.warn("Enter an HTTPS endpoint or a localhost HTTP endpoint");
                    return;
                }
                if draft.endpoint != value {
                    draft.endpoint = value;
                    draft.method = AuthMethod::None;
                    draft.token.clear();
                    draft.oauth = None;
                    draft.oauth_client_id = None;
                    draft.headers.clear();
                    self.mcp.methods.clear();
                }
            }
            Field::Token => {
                draft.method = AuthMethod::Token;
                draft.token = value;
                draft.oauth = None;
            }
            Field::Timeout => match value.parse::<u64>() {
                Ok(seconds @ 1..=600) => draft.timeout_secs = seconds,
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
                draft
                    .headers
                    .retain(|(key, _)| !key.eq_ignore_ascii_case(name.trim()));
                if !value.trim().is_empty() {
                    draft
                        .headers
                        .push((name.trim().into(), value.trim().into()));
                }
                self.mcp.methods.clear();
                self.mcp.method_cache.clear();
            }
            Field::ClientId => draft.oauth_client_id = (!value.is_empty()).then_some(value),
        }
        draft.protect(&self.archive.redactor);
        if matches!(field, Field::Timeout | Field::Header | Field::ClientId) {
            self.show_mcp_advanced();
        } else {
            self.show_mcp_form();
        }
    }
}
