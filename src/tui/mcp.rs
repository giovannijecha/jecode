//! Global MCP configuration and read-only inspection in the B5 selector.
mod fields;
mod flow;
mod health;
#[cfg(test)]
mod integration_tests;
mod job;
pub(super) mod menus;
#[cfg(test)]
mod navigation_tests;
mod results;
mod sign_in;
#[cfg(test)]
mod tests;

pub(super) use health::Health;

use super::{
    App,
    selector::{Purpose, Selector},
};
use crate::mcp::{AuthMethod, Catalog, Connection, ErrorKind};
use job::{Checks, Job, ResultValue};
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
    Detail(Health),
    Form,
    Field(Field),
    Methods,
    Advanced,
    ConfirmRemove,
    Tools,
    Loading,
}

/// One unsaved connection and everything learned about it, replaced as a whole.
pub(super) struct Draft {
    connection: Connection,
    /// The saved connection being edited; `None` for a new one.
    original: Option<Connection>,
    /// Sign-in methods offered by the draft's endpoint, once checked.
    methods: Vec<AuthMethod>,
    /// Started by a saved connection's Sign in action, which saves on success.
    sign_in: bool,
}

#[derive(Default)]
pub(super) struct Ui {
    open: bool,
    settings: bool,
    from_inspection: bool,
    connections: Vec<Connection>,
    health: HashMap<String, Result<Catalog, ErrorKind>>,
    checks: Checks,
    selected: Option<String>,
    draft: Option<Draft>,
    method_cache: HashMap<String, Vec<AuthMethod>>,
    tool: usize,
    job: Option<Job>,
    /// The panel Esc, or a failed worker, returns to from a loading panel.
    back: Option<Panel>,
    updates: Option<Receiver<crate::mcp::auth::Update>>,
    browser_url: Option<String>,
    device_code: Option<String>,
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

    fn connection(&self) -> Option<&Connection> {
        self.draft.as_ref().map(|draft| &draft.connection)
    }
}

/// The display name of the connection an `mcp` call names, by ID or name.
pub(super) fn label(connections: &[Connection], arguments: &crate::json::Value) -> Option<String> {
    let target = arguments.get("connection")?.as_str()?;
    connections
        .iter()
        .find(|c| c.id == target)
        .or_else(|| {
            connections
                .iter()
                .find(|c| c.name.eq_ignore_ascii_case(target))
        })
        .map(|c| c.name.clone())
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
        self.mcp.job = Some(Job::list(self.mcp_home()));
        self.mcp_loading("Loading configured connections…", Panel::List);
    }

    /// Begin a draft, resetting everything learned about a previous one.
    fn begin_mcp_draft(&mut self, connection: Connection, saved: bool, sign_in: bool) {
        let methods = self
            .mcp
            .method_cache
            .get(&Ui::methods_key(&connection))
            .cloned()
            .unwrap_or_default();
        if !saved {
            self.mcp.selected = None;
        }
        self.mcp.draft = Some(Draft {
            original: saved.then(|| connection.clone()),
            connection,
            methods,
            sign_in,
        });
    }

    fn show_mcp_panel(&mut self, panel: Panel) {
        match panel {
            Panel::List | Panel::Loading => self.show_mcp_list(),
            Panel::Add => self.show_mcp_add(),
            Panel::Detail(_) => self.show_mcp_detail(),
            Panel::Form => self.show_mcp_form(),
            Panel::Field(field) => self.show_mcp_field(field),
            Panel::Methods => self.show_mcp_methods(),
            Panel::Advanced => self.show_mcp_advanced(),
            Panel::ConfirmRemove => self.show_mcp_remove(),
            Panel::Tools => self.show_mcp_tools(),
        }
    }

    fn show_mcp_list(&mut self) {
        let index = |id: &String| self.mcp.connections.iter().position(|c| &c.id == id);
        // Returning from a connection or the add panel focuses where it came from.
        let focus = match self
            .state
            .selector
            .as_ref()
            .map(|m| (&m.purpose, m.selected))
        {
            Some((Purpose::Mcp(Panel::List), selected)) => Some(selected),
            Some((Purpose::Mcp(Panel::Add), _)) => Some(self.mcp.connections.len()),
            _ => self.mcp.selected.as_ref().and_then(index),
        };
        let rows = self
            .mcp
            .connections
            .iter()
            .map(|c| (c.name.clone(), Health::of(c, &self.mcp)))
            .collect::<Vec<_>>();
        let mut menu = menus::list(&rows, self.mcp.settings);
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
        self.state.selector = Some(menus::add());
    }

    fn show_mcp_detail(&mut self) {
        let Some(connection) = self.selected_mcp() else {
            self.show_mcp_list();
            return;
        };
        let menu = menus::detail(connection, Health::of(connection, &self.mcp));
        self.state.selector = Some(menu);
    }

    fn show_mcp_remove(&mut self) {
        let Some(connection) = self.selected_mcp() else {
            self.show_mcp_list();
            return;
        };
        self.state.selector = Some(Selector::mcp(
            Panel::ConfirmRemove,
            "Remove connection?",
            Some("MCP"),
            "sign-in removed",
            vec![("Remove".into(), connection.name.clone())],
        ));
    }

    fn show_mcp_form(&mut self) {
        let Some(draft) = &self.mcp.draft else {
            self.show_mcp_detail();
            return;
        };
        self.state.selector = Some(menus::form(&draft.connection, &draft.methods));
    }

    fn show_mcp_advanced(&mut self) {
        let Some(connection) = self.mcp.connection() else {
            self.show_mcp_detail();
            return;
        };
        self.state.selector = Some(menus::advanced(connection));
    }

    pub(super) fn choose_mcp(&mut self, panel: Panel, index: usize) {
        match panel {
            Panel::List => self.choose_mcp_connection(index),
            Panel::Add => {
                let (name, endpoint, _) = PRESETS.get(index).copied().unwrap_or(("", "", ""));
                self.begin_mcp_draft(Connection::draft(name, endpoint), false, false);
                if name.is_empty() {
                    self.show_mcp_field(Field::Endpoint);
                } else {
                    self.show_mcp_form();
                }
            }
            Panel::Detail(_) => self.choose_mcp_detail(index),
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

    fn choose_mcp_connection(&mut self, index: usize) {
        if index == self.mcp.connections.len() && self.mcp.settings {
            self.show_mcp_add();
            return;
        }
        let Some(c) = self.mcp.connections.get(index) else {
            return;
        };
        self.mcp.selected = Some(c.id.clone());
        let healthy = self.mcp.health.get(&c.id).is_some_and(Result::is_ok);
        if !self.mcp.settings && c.enabled && !self.mcp.health.contains_key(&c.id) {
            self.mcp.probe_inspection = true;
            self.start_mcp_probe(c.clone());
        } else if self.mcp.settings || !c.enabled || !healthy {
            if !self.mcp.settings {
                self.mcp.settings = true;
                self.mcp.from_inspection = true;
            }
            self.show_mcp_detail();
        } else {
            self.show_mcp_tools();
        }
    }

    fn choose_mcp_detail(&mut self, index: usize) {
        let Some(c) = self.selected_mcp().cloned() else {
            return;
        };
        match index {
            0 => {
                self.begin_mcp_draft(c, true, false);
                self.show_mcp_form();
            }
            1 => {
                self.begin_mcp_draft(c, true, true);
                self.start_mcp_methods();
            }
            2 => self.start_mcp_probe(c),
            3 => {
                self.mcp.job = Some(Job::enabled(self.mcp_home(), c.id, !c.enabled));
                let message = if c.enabled {
                    "Suspending connection…"
                } else {
                    "Resuming connection…"
                };
                self.mcp_loading(message, Panel::Detail(Health::Unchecked));
            }
            4 => self.show_mcp_remove(),
            _ => {}
        }
    }
}
