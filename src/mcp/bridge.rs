//! One neutral tool bridge; remote schemas are discovered only when needed.

use super::{Client, Connection, Error, ErrorKind, Runtime, Store, Tool};
use crate::{cancel::Cancellation, json::Value, output, redact::Redactor};
use std::{
    path::Path,
    sync::{Mutex, MutexGuard, TryLockError},
    time::Duration,
};

mod bounded;
#[cfg(test)]
mod stepup_tests;
#[cfg(test)]
mod tests;

const CONFIGURATION: &str = "Settings > MCP in the TUI, or ~/.jecode/mcp.json";

pub fn include_credentials(home: &Path, redactor: &Redactor) -> Result<(), String> {
    for connection in Store::new(home.to_path_buf()).list().map_err(|error| {
        format!(
            "Could not read MCP connections from {}: {error}",
            home.join("mcp.json").display()
        )
    })? {
        connection.protect(redactor);
    }
    Ok(())
}

/// The display name of the connection an `mcp` call targets.
pub fn connection_name(connections: &[Connection], arguments: &Value) -> Option<String> {
    let target = arguments.get("connection")?.as_str()?;
    find(connections, target).map(|connection| connection.name.clone())
}

/// The connection's display name; registers saved credentials from the same read.
pub fn protected_label(home: &Path, arguments: &Value, redactor: &Redactor) -> Option<String> {
    let connections = Store::new(home.to_path_buf()).list().ok()?;
    for connection in &connections {
        connection.protect(redactor);
    }
    connection_name(&connections, arguments)
}

/// Scheme and host only: hosted endpoints often embed credentials in the
/// user information, path or query.
pub fn origin(endpoint: &str) -> String {
    let (scheme, rest) = endpoint
        .split_once("://")
        .map_or(("", endpoint), |(scheme, rest)| (scheme, rest));
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    if scheme.is_empty() {
        host.into()
    } else {
        format!("{scheme}://{host}")
    }
}

/// The endpoint without its query or fragment, for the user's own output.
pub fn without_query(endpoint: &str) -> &str {
    endpoint.split(['?', '#']).next().unwrap_or(endpoint)
}

#[cfg(test)]
pub fn execute(
    home: Option<&Path>,
    arguments: &Value,
    cancel: &Cancellation,
    redactor: &Redactor,
) -> Value {
    execute_with(&Runtime::default(), home, arguments, cancel, redactor, None)
}

pub fn execute_with(
    runtime: &Runtime,
    home: Option<&Path>,
    arguments: &Value,
    cancel: &Cancellation,
    redactor: &Redactor,
    outputs: Option<&output::Store>,
) -> Value {
    let result = run(runtime, home, arguments, cancel, redactor);
    bounded::bound(
        redactor.value(&result.unwrap_or_else(|error| {
            let required_scope = super::auth::scopes::required(&error);
            let mut fields = vec![
                ("error", Value::string(error.message)),
                (
                    "outcome",
                    Value::string(match error.kind {
                        ErrorKind::UnknownOutcome => "unknown",
                        ErrorKind::Cancelled => "not_executed",
                        _ => "failed",
                    }),
                ),
                (
                    "sign_in_required",
                    Value::Bool(error.kind == ErrorKind::Authentication),
                ),
            ];
            if let Some(scope) = required_scope {
                fields.push(("required_scope", Value::string(scope)));
            }
            Value::Object(
                fields
                    .into_iter()
                    .map(|(name, value)| (name.into(), value))
                    .collect(),
            )
        })),
        outputs,
    )
}

fn run(
    runtime: &Runtime,
    home: Option<&Path>,
    arguments: &Value,
    cancel: &Cancellation,
    redactor: &Redactor,
) -> Result<Value, Error> {
    let Some(home) = home else {
        return Err(Error::new(
            ErrorKind::Configuration,
            "MCP connections are unavailable without a configuration directory",
        ));
    };
    let store = Store::new(home.to_path_buf());
    let connections = store.list()?;
    runtime.prune(home, &connections)?;
    for connection in &connections {
        connection.protect(redactor);
    }
    let action = arguments
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("list");
    if action == "list" {
        return Ok(Value::object([
            (
                "connections",
                Value::Array(
                    connections
                        .iter()
                        .map(|c| {
                            Value::object([
                                ("id", Value::string(&c.id)),
                                ("name", Value::string(&c.name)),
                                ("origin", Value::string(origin(&c.endpoint))),
                                ("enabled", Value::Bool(c.enabled)),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "configuration",
                Value::string(format!("Global connections are managed in {CONFIGURATION}")),
            ),
        ]));
    }
    if !matches!(action, "tools" | "call") {
        return Err(Error::new(
            ErrorKind::Configuration,
            "MCP action must be list, tools or call",
        ));
    }
    let target = text(arguments, "connection")?;
    let saved = find(&connections, target).ok_or_else(|| {
        Error::new(
            ErrorKind::Configuration,
            "MCP connection not found; list configured connections first",
        )
    })?;
    if !saved.enabled {
        return Err(Error::new(
            ErrorKind::Configuration,
            format!(
                "{} is suspended globally; resume it in {CONFIGURATION}",
                saved.name
            ),
        ));
    }
    let not_sent = || Error::new(ErrorKind::Cancelled, "MCP call was not sent");
    if cancel.requested() {
        return Err(not_sent());
    }
    // Refresh under the client lock, so a queued call never dispatches a token
    // that expired while it waited.
    let shared = runtime.client(home, saved)?;
    let mut client = acquire(&shared, cancel).map_err(|error| error.unwrap_or_else(not_sent))?;
    let connection = store.fresh(&saved.id, cancel)?;
    connection.protect(redactor);
    if !same_target(saved, &connection) {
        return Err(Error::new(
            ErrorKind::Configuration,
            "MCP connection changed or was suspended before dispatch; inspect it again",
        ));
    }
    runtime.bind(home, &shared, &mut client, &connection)?;
    if cancel.requested() {
        return Err(not_sent());
    }
    if action == "tools" {
        let catalog = match client.catalog(cancel) {
            Ok(catalog) => catalog,
            Err(error) => {
                *client = Client::new(connection.clone());
                remember_scope(&store, &connection, &error)?;
                return Err(error);
            }
        };
        return Ok(discovered(&connection, &catalog, arguments));
    }
    let name = text(arguments, "tool")?;
    let input = match arguments.get("arguments") {
        None => Value::object([]),
        // Some models encode the object as a JSON string.
        Some(Value::String(encoded)) => crate::json::parse(encoded)
            .ok()
            .filter(|value| matches!(value, Value::Object(_)))
            .unwrap_or_else(|| Value::string(encoded)),
        Some(value) => value.clone(),
    };
    if !matches!(input, Value::Object(_)) {
        return Err(Error::new(
            ErrorKind::Configuration,
            "MCP tool arguments must be a JSON object",
        ));
    }
    let result = match client.call(name, &input, cancel) {
        Ok(result) => result,
        Err(error) => {
            *client = Client::new(connection.clone());
            remember_scope(&store, &connection, &error)?;
            return Err(error);
        }
    };
    let failed = result.get("isError") == Some(&Value::Bool(true));
    let mut value = Value::object([
        ("connection", identity(&connection)),
        ("tool", Value::string(name)),
        (
            "outcome",
            Value::string(if failed { "failed" } else { "completed" }),
        ),
        ("result", result),
    ]);
    if failed && let Value::Object(fields) = &mut value {
        fields.insert(
            "error".into(),
            Value::string("The MCP server reported a tool error"),
        );
    }
    Ok(value)
}

/// A challenged tool is never replayed. Sign-in can request the scope later.
fn remember_scope(store: &Store, connection: &Connection, error: &Error) -> Result<(), Error> {
    if connection.method == super::AuthMethod::Browser
        && let Some(scope) = super::auth::scopes::required(error)
    {
        store.require_scope(connection, &scope)?;
    }
    Ok(())
}

/// Waits for the connection's session without ignoring cancellation;
/// `Err(None)` means the wait was cancelled.
fn acquire<'a>(
    shared: &'a Mutex<Client>,
    cancel: &Cancellation,
) -> Result<MutexGuard<'a, Client>, Option<Error>> {
    loop {
        match shared.try_lock() {
            Ok(client) => return Ok(client),
            Err(TryLockError::Poisoned(_)) => return Err(Some(super::runtime::poisoned())),
            Err(TryLockError::WouldBlock) => {
                cancel.wait(Duration::from_millis(20)).map_err(|_| None)?
            }
        }
    }
}

/// Token renewal changes only the OAuth record; anything else changed the target.
fn same_target(saved: &Connection, current: &Connection) -> bool {
    let mut saved = saved.clone();
    saved.oauth.clone_from(&current.oauth);
    saved == *current
}

fn discovered(connection: &Connection, catalog: &super::Catalog, arguments: &Value) -> Value {
    let query = arguments
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    let matching = catalog
        .tools
        .iter()
        .filter(|tool| {
            tool.name.to_lowercase().contains(&query)
                || tool.description.to_lowercase().contains(&query)
        })
        .collect::<Vec<_>>();
    let offset = arguments
        .get("offset")
        .and_then(Value::as_usize)
        .unwrap_or(0);
    let limit = arguments
        .get("limit")
        .and_then(Value::as_usize)
        .unwrap_or(8)
        .clamp(1, 50);
    let tools = matching
        .iter()
        .skip(offset)
        .take(limit)
        .map(|tool| definition(tool))
        .collect::<Vec<_>>();
    let end = offset.saturating_add(tools.len());
    Value::object([
        ("connection", identity(connection)),
        (
            "server",
            Value::object([
                ("name", Value::string(&catalog.server.name)),
                ("version", Value::string(&catalog.server.version)),
                (
                    "instructions",
                    catalog
                        .server
                        .instructions
                        .as_deref()
                        .map_or(Value::Null, Value::string),
                ),
            ]),
        ),
        ("tools", Value::Array(tools)),
        ("total", Value::number(matching.len())),
        (
            "next_offset",
            if end < matching.len() {
                Value::number(end)
            } else {
                Value::Null
            },
        ),
    ])
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str, Error> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            Error::new(
                ErrorKind::Configuration,
                format!("MCP {key} must be a nonempty string"),
            )
        })
}

fn find<'a>(connections: &'a [Connection], target: &str) -> Option<&'a Connection> {
    connections
        .iter()
        .find(|connection| connection.id == target)
        .or_else(|| {
            connections
                .iter()
                .find(|connection| connection.name.eq_ignore_ascii_case(target))
        })
}

fn identity(connection: &Connection) -> Value {
    Value::object([
        ("id", Value::string(&connection.id)),
        ("name", Value::string(&connection.name)),
    ])
}

fn definition(tool: &Tool) -> Value {
    Value::object([
        ("name", Value::string(&tool.name)),
        (
            "title",
            tool.title.as_deref().map_or(Value::Null, Value::string),
        ),
        ("description", Value::string(&tool.description)),
        ("inputSchema", tool.input_schema.clone()),
        (
            "annotations",
            tool.annotations.clone().unwrap_or(Value::Null),
        ),
    ])
}

pub fn summary(result: &Value) -> String {
    if result.get("outcome").and_then(Value::as_str) == Some("unknown") {
        return "Outcome unknown · check the remote state before retrying".into();
    }
    if result.get("outcome").and_then(Value::as_str) == Some("not_executed") {
        return "Cancelled · call was not sent".into();
    }
    let content = result
        .get("result")
        .and_then(|value| value.get("content"))
        .and_then(Value::as_array)
        .and_then(|blocks| {
            blocks.iter().find_map(|block| {
                block
                    .get("text")
                    .and_then(Value::as_str)
                    .and_then(|text| text.lines().find(|line| !line.trim().is_empty()))
            })
        });
    if let Some(error) = result.get("error").and_then(Value::as_str) {
        return content
            .unwrap_or_else(|| error.lines().next().unwrap_or(error))
            .chars()
            .take(240)
            .collect();
    }
    if result.get("preview").is_some() && result.get("truncated") == Some(&Value::Bool(true)) {
        return "Output truncated · read result_file".into();
    }
    if let Some(connections) = result.get("connections").and_then(Value::as_array) {
        return format!("{} configured connections", connections.len());
    }
    if let Some(tools) = result.get("tools").and_then(Value::as_array) {
        let total = result
            .get("total")
            .and_then(Value::as_usize)
            .unwrap_or(tools.len());
        return if total > tools.len() {
            format!("{} of {total} tools", tools.len())
        } else {
            format!("{total} tools")
        };
    }
    if let Some(text) = content {
        return text.chars().take(240).collect();
    }
    "Completed".into()
}
