//! B14 panels built from connection state, shared by the app and its specimens.
use super::{AuthMethod, Connection, Health, PRESETS, Panel};
use crate::tui::selector::{Choice, Selector};

/// Configured connections with their states; Settings adds `Add connection`.
pub(in crate::tui) fn list(rows: &[(String, Health)], settings: bool) -> Selector {
    let available = rows
        .iter()
        .filter(|(_, health)| matches!(health, Health::Available(_)))
        .count();
    let mut options = rows
        .iter()
        .map(|(name, health)| (name.clone(), health.fact()))
        .collect::<Vec<_>>();
    if settings {
        options.push(("Add connection".into(), String::new()));
    }
    let mut menu = Selector::mcp(
        Panel::List,
        "MCP",
        settings.then_some("Settings"),
        "all conversations",
        options,
    );
    for (option, (_, health)) in menu.options.iter_mut().zip(rows) {
        if let Choice::Mcp(index, _) = option.choice {
            option.choice = Choice::Mcp(index, Some(*health));
        }
    }
    menu.message = if rows.is_empty() && !settings {
        "No configured connections · open Settings › MCP".into()
    } else {
        format!("{available} available · {} configured", rows.len())
    };
    menu
}

/// Guided examples, then any server by URL.
pub(in crate::tui) fn add() -> Selector {
    let mut rows = PRESETS
        .iter()
        .map(|(name, _, fact)| ((*name).into(), (*fact).into()))
        .collect::<Vec<_>>();
    rows.push(("Server URL".into(), "Any HTTP MCP server".into()));
    Selector::mcp(Panel::Add, "Add connection", Some("MCP"), "", rows)
}

pub(in crate::tui) fn detail(connection: &Connection, health: Health) -> Selector {
    let rows = vec![
        ("Edit".into(), String::new()),
        ("Sign in".into(), String::new()),
        ("Reconnect".into(), String::new()),
        (
            if connection.enabled {
                "Suspend"
            } else {
                "Resume"
            }
            .into(),
            String::new(),
        ),
        ("Remove".into(), String::new()),
    ];
    let mut menu = Selector::mcp(
        Panel::Detail(health),
        &connection.name,
        Some("MCP"),
        "all conversations",
        rows,
    );
    menu.message = format!("{}\n{}", health.state(), connection.endpoint);
    menu
}

pub(in crate::tui) fn form(connection: &Connection, methods: &[AuthMethod]) -> Selector {
    let method = match connection.method {
        AuthMethod::None if methods == [AuthMethod::None] => "Not needed",
        AuthMethod::None => "Choose Token or OAuth",
        AuthMethod::Token if connection.token.is_empty() => "Token · not entered",
        AuthMethod::Token => "Token · entered, hidden",
        AuthMethod::Browser | AuthMethod::GitHubDevice if connection.oauth.is_none() => {
            "OAuth · not signed in"
        }
        AuthMethod::Browser | AuthMethod::GitHubDevice => "OAuth",
    };
    let rows = vec![
        ("Name".into(), connection.name.clone()),
        ("Endpoint".into(), connection.endpoint.clone()),
        ("Sign-in".into(), method.into()),
        ("Advanced".into(), String::new()),
        ("Save connection".into(), String::new()),
    ];
    Selector::mcp(
        Panel::Form,
        "Connection",
        Some("MCP"),
        "all conversations",
        rows,
    )
}

/// Jecode bundles its own OAuth client only for GitHub; every other endpoint,
/// guided or not, may need an approved client ID.
pub(super) fn bundled_client(connection: &Connection) -> bool {
    connection.endpoint.starts_with(PRESETS[0].1)
}

pub(in crate::tui) fn advanced(connection: &Connection) -> Selector {
    let mut rows = vec![
        (
            "Timeout".into(),
            format!("{} seconds", connection.timeout_secs),
        ),
        (
            "Extra header".into(),
            format!("{} configured, hidden", connection.headers.len()),
        ),
    ];
    if !bundled_client(connection) {
        rows.push((
            "Custom OAuth client".into(),
            connection
                .oauth_client_id
                .as_ref()
                .map_or("Automatic".into(), |_| "Configured".into()),
        ));
    }
    Selector::mcp(Panel::Advanced, "Advanced", Some("Connection"), "", rows)
}

pub(in crate::tui) fn methods(methods: &[AuthMethod]) -> Selector {
    let rows = methods
        .iter()
        .map(|method| {
            let (name, fact) = match method {
                AuthMethod::None => ("Not needed", "Public server"),
                AuthMethod::Token => ("Token", "Paste an access token"),
                AuthMethod::Browser | AuthMethod::GitHubDevice => ("OAuth", "Open browser"),
            };
            (name.into(), fact.into())
        })
        .collect();
    let mut menu = Selector::mcp(
        Panel::Methods,
        "Sign-in",
        Some("Connection"),
        "server options",
        rows,
    );
    if methods.is_empty() {
        menu.message = "No sign-in method offered by this server".into();
    }
    menu
}
