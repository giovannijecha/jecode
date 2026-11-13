//! `jecode mcp`: the connections Settings > MCP manages, through the same
//! store, sign-in and check. Listing and showing read only the local file;
//! `tools`, `add` and `login` contact the server because they are asked to.
//! MCP access follows this enablement, independently of local levels.

use super::{Context, Report, confirm, secret};
use crate::cancel::Cancellation;
use crate::cli::{Failure, Manage, usage};
use crate::json::Value;
use crate::mcp::{self, AuthMethod, Connection, Store};
use crate::redact::Redactor;
use std::path::Path;

pub fn run(context: &Context, command: &Manage) -> Result<Report, Failure> {
    let store = Store::new(context.home.clone());
    match command.action {
        "list" => list(&store),
        "show" => {
            let connection = find(&store, &command.operands[0])?;
            Ok(Report::new(value(&connection), detail(&connection)))
        }
        "tools" => tools(&store, &command.operands[0]),
        "add" => add(
            &store,
            &command.operands[0],
            &command.operands[1],
            command.value("auth").unwrap_or("none"),
        ),
        "login" => login(&store, &command.operands[0]),
        "enable" | "disable" => {
            let connection = find(&store, &command.operands[0])?;
            let enabled = command.action == "enable";
            store.set_enabled(&connection.id, enabled).map_err(text)?;
            Ok(Report::new(
                Value::object([
                    ("id", Value::string(&connection.id)),
                    ("enabled", Value::Bool(enabled)),
                ]),
                format!(
                    "{} {} for every conversation",
                    connection.name,
                    if enabled { "enabled" } else { "suspended" }
                ),
            ))
        }
        "remove" => {
            let connection = find(&store, &command.operands[0])?;
            if !confirm(
                command.flag("yes"),
                &format!(
                    "Remove MCP connection {}? Its saved credentials are deleted.",
                    connection.name
                ),
                "remove",
            )? {
                return Err("Cancelled; the connection was kept".into());
            }
            store.remove(&connection.id).map_err(text)?;
            Ok(Report::new(
                Value::object([
                    ("id", Value::string(&connection.id)),
                    ("removed", Value::Bool(true)),
                ]),
                format!("Removed {}", connection.name),
            ))
        }
        action => unreachable!("mcp {action}"),
    }
}

fn text(error: mcp::Error) -> Failure {
    Failure::Error(error.message)
}

fn find(store: &Store, target: &str) -> Result<Connection, Failure> {
    super::resolve(
        store
            .list()
            .map_err(text)?
            .into_iter()
            .map(|connection| (connection.id.clone(), connection.name.clone(), connection)),
        target,
        "MCP connection",
        "mcp",
    )
}

fn method(connection: &Connection) -> &'static str {
    match connection.method {
        AuthMethod::None => "none",
        AuthMethod::Token => "token",
        AuthMethod::Browser => "oauth",
        AuthMethod::GitHubDevice => "oauth (GitHub device)",
    }
}

fn signed_in(connection: &Connection) -> &'static str {
    match connection.method {
        AuthMethod::None => "not needed",
        AuthMethod::Token if connection.token.is_empty() => "no token",
        AuthMethod::Token => "token saved",
        _ if connection.oauth.is_some() => "signed in",
        _ => "signed out",
    }
}

fn value(connection: &Connection) -> Value {
    Value::object([
        ("id", Value::string(&connection.id)),
        ("name", Value::string(&connection.name)),
        (
            "endpoint",
            Value::string(mcp::without_query(&connection.endpoint)),
        ),
        ("enabled", Value::Bool(connection.enabled)),
        ("auth", Value::string(method(connection))),
        ("credentials", Value::string(signed_in(connection))),
        ("timeout_secs", Value::number(connection.timeout_secs)),
        (
            "headers",
            Value::Array(
                connection
                    .headers
                    .iter()
                    .map(|(name, _)| Value::string(name))
                    .collect(),
            ),
        ),
    ])
}

fn detail(connection: &Connection) -> String {
    let headers = connection
        .headers
        .iter()
        .map(|(name, _)| format!("{name}: ••••"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{} ({})\nEndpoint: {}\nState:    {}\nSign-in:  {} · {}\nTimeout:  {} s\nHeaders:  {}",
        connection.name,
        connection.id,
        mcp::without_query(&connection.endpoint),
        if connection.enabled {
            "enabled"
        } else {
            "suspended"
        },
        method(connection),
        signed_in(connection),
        connection.timeout_secs,
        if headers.is_empty() { "none" } else { &headers }
    )
}

/// The configured connections without contacting them: the plain `/mcp`
/// view and `jecode mcp list` share it.
pub(crate) fn overview(home: &Path) -> (String, String) {
    let connections = match Store::new(home.to_path_buf()).list() {
        Ok(connections) => connections,
        Err(error) => {
            let error = format!("Could not read MCP connections: {error}");
            return (error.clone(), error);
        }
    };
    let mut text = connections
        .iter()
        .map(|connection| {
            format!(
                "{} · {} · {}\n",
                connection.name,
                mcp::without_query(&connection.endpoint),
                if connection.enabled {
                    "enabled"
                } else {
                    "suspended"
                }
            )
        })
        .collect::<String>();
    text.push_str(
        "Manage connections with jecode mcp, in Settings > MCP in the TUI, or in ~/.jecode/mcp.json.",
    );
    (
        format!("{} configured connections", connections.len()),
        text,
    )
}

fn list(store: &Store) -> Result<Report, Failure> {
    let connections = store.list().map_err(text)?;
    let mut text = String::new();
    for connection in &connections {
        text.push_str(&format!(
            "{} ({}) · {} · {} · {}\n",
            connection.name,
            connection.id,
            mcp::without_query(&connection.endpoint),
            if connection.enabled {
                "enabled"
            } else {
                "suspended"
            },
            signed_in(connection)
        ));
    }
    if connections.is_empty() {
        text.push_str("No MCP connections. Add one with jecode mcp add NAME URL.\n");
    }
    text.push_str("Nothing was contacted; jecode mcp tools CONNECTION checks one.");
    Ok(Report::new(
        Value::object([(
            "connections",
            Value::Array(connections.iter().map(value).collect()),
        )]),
        text,
    ))
}

fn redactor(connection: &Connection) -> Redactor {
    let redactor = Redactor::empty();
    connection.protect(&redactor);
    redactor
}

fn catalog(connection: Connection) -> Result<mcp::Catalog, Failure> {
    let redactor = redactor(&connection);
    mcp::Client::new(connection)
        .catalog(&Cancellation::default())
        .map_err(|error| Failure::Error(redactor.text(&error.message)))
}

fn tools(store: &Store, target: &str) -> Result<Report, Failure> {
    let saved = find(store, target)?;
    if !saved.enabled {
        return Err(Failure::Error(format!(
            "{} is suspended; enable it with jecode mcp enable {}",
            saved.name, saved.id
        )));
    }
    let connection = store
        .fresh(&saved.id, &Cancellation::default())
        .map_err(|error| Failure::Error(redactor(&saved).text(&error.message)))?;
    let catalog = catalog(connection.clone())?;
    let redactor = redactor(&connection);
    let mut text = format!(
        "{} · {} {} · {} tools\n",
        connection.name,
        catalog.server.name,
        catalog.server.version,
        catalog.tools.len()
    );
    for tool in &catalog.tools {
        text.push_str(&format!(
            "{}  {}\n",
            tool.name,
            tool.description.lines().next().unwrap_or_default()
        ));
    }
    let value = Value::object([
        ("id", Value::string(&connection.id)),
        ("server", Value::string(&catalog.server.name)),
        (
            "tools",
            Value::Array(
                catalog
                    .tools
                    .iter()
                    .map(|tool| {
                        Value::object([
                            ("name", Value::string(&tool.name)),
                            ("description", Value::string(&tool.description)),
                            ("input_schema", tool.input_schema.clone()),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]);
    Ok(Report::new(redactor.value(&value), redactor.text(&text)))
}

fn oauth_method(connection: &Connection) -> Result<AuthMethod, Failure> {
    let methods = mcp::auth::methods(connection, &Cancellation::default()).map_err(text)?;
    if methods.contains(&AuthMethod::GitHubDevice) {
        Ok(AuthMethod::GitHubDevice)
    } else if methods.contains(&AuthMethod::Browser) {
        Ok(AuthMethod::Browser)
    } else {
        Err(Failure::Error(
            "The server does not offer OAuth sign-in; use --auth token or none".into(),
        ))
    }
}

fn browser_sign_in(connection: Connection) -> Result<Connection, Failure> {
    let redactor = redactor(&connection);
    mcp::auth::sign_in(connection, &Cancellation::default(), &mut |update| {
        match update {
            mcp::auth::Update::Browser(url) => eprintln!("Continue in your browser:\n{url}"),
            mcp::auth::Update::DeviceCode { url, code } => {
                eprintln!("Open {url} and enter the code {code}")
            }
            mcp::auth::Update::Waiting => eprintln!("Waiting for sign-in…"),
        }
        Ok(())
    })
    .map_err(|error| Failure::Error(redactor.text(&error.message)))
}

fn add(store: &Store, name: &str, url: &str, auth: &str) -> Result<Report, Failure> {
    mcp::config::validate_url(url).map_err(|error| usage(error.message))?;
    let mut connection = Connection::draft(name.trim(), url.trim());
    mcp::config::validate(&connection).map_err(|error| usage(error.message))?;
    if store
        .list()
        .map_err(text)?
        .iter()
        .any(|existing| existing.name.eq_ignore_ascii_case(name.trim()))
    {
        return Err(usage(format!(
            "an MCP connection named {name} already exists"
        )));
    }
    match auth {
        "token" => {
            connection.method = AuthMethod::Token;
            connection.token = secret("Bearer token (hidden): ")?;
            // Validate the token before it reaches a request header.
            mcp::config::validate(&connection).map_err(|error| usage(error.message))?;
        }
        "oauth" => {
            connection.method = oauth_method(&connection)?;
            connection = browser_sign_in(connection)?;
        }
        _ => {}
    }
    eprintln!("Checking the connection and loading tools…");
    let catalog = catalog(connection.clone())?;
    let saved = store.save(&connection).map_err(text)?;
    Ok(Report::new(
        value(&saved),
        format!(
            "Saved {} ({}) · {} tools · enabled for every conversation",
            saved.name,
            saved.id,
            catalog.tools.len()
        ),
    ))
}

fn login(store: &Store, target: &str) -> Result<Report, Failure> {
    let original = find(store, target)?;
    let (signed, guard) = match original.method {
        AuthMethod::None => {
            return Err(Failure::Error(format!(
                "{} uses no sign-in; remove and add it again with --auth to change that",
                original.name
            )));
        }
        AuthMethod::Token => {
            let mut next = original.clone();
            next.token = secret("Bearer token (hidden): ")?;
            mcp::config::validate(&next).map_err(|error| usage(error.message))?;
            (next, None)
        }
        AuthMethod::Browser | AuthMethod::GitHubDevice => {
            let (prepared, guard) = store
                .prepare_sign_in(&original, Some(&original))
                .map_err(text)?;
            (browser_sign_in(prepared)?, guard)
        }
    };
    eprintln!("Checking the connection and loading tools…");
    let catalog = catalog(signed.clone())?;
    let saved = match &guard {
        Some(guard) => store.commit_sign_in(&signed, guard),
        None => store.save(&signed),
    }
    .map_err(text)?;
    Ok(Report::new(
        value(&saved),
        format!("{} signed in · {} tools", saved.name, catalog.tools.len()),
    ))
}
