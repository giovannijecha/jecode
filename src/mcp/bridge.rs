//! One neutral tool bridge; remote schemas are discovered only when needed.

use super::{Client, Connection, Error, ErrorKind, Runtime, Store, Tool};
use crate::{cancel::Cancellation, json::Value, redact::Redactor};
use std::path::Path;

pub fn include_credentials(home: &Path, redactor: &Redactor) -> Result<(), String> {
    for connection in Store::new(home.to_path_buf())
        .list()
        .map_err(|e| e.to_string())?
    {
        connection.protect(redactor);
    }
    Ok(())
}

pub fn label(home: Option<&Path>, arguments: &Value) -> Option<String> {
    let target = arguments.get("connection")?.as_str()?;
    let connections = Store::new(home?.to_path_buf()).list().ok()?;
    find(&connections, target).map(|connection| connection.name.clone())
}

pub fn execute(
    home: Option<&Path>,
    arguments: &Value,
    cancel: &Cancellation,
    redactor: &Redactor,
) -> Value {
    execute_with(&Runtime::default(), home, arguments, cancel, redactor)
}

pub fn execute_with(
    runtime: &Runtime,
    home: Option<&Path>,
    arguments: &Value,
    cancel: &Cancellation,
    redactor: &Redactor,
) -> Value {
    let result = run(runtime, home, arguments, cancel, redactor);
    redactor.value(&result.unwrap_or_else(|error| {
        Value::object([
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
        ])
    }))
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
                                ("endpoint", Value::string(&c.endpoint)),
                                ("enabled", Value::Bool(c.enabled)),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "configuration",
                Value::string("Global connections are managed in /settings > MCP"),
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
                "{} is suspended globally; resume it in /settings > MCP",
                saved.name
            ),
        ));
    }
    if cancel.requested() {
        return Err(Error::new(ErrorKind::Cancelled, "MCP call was not sent"));
    }
    let connection = store.fresh(&saved.id, cancel)?;
    connection.protect(redactor);
    let shared = runtime.client(home, &connection)?;
    let mut client = shared.lock().map_err(|_| super::runtime::poisoned())?;
    if cancel.requested() {
        return Err(Error::new(ErrorKind::Cancelled, "MCP call was not sent"));
    }
    if !store
        .list()?
        .iter()
        .any(|current| current == &connection && current.enabled)
    {
        return Err(Error::new(
            ErrorKind::Configuration,
            "MCP connection changed or was suspended before dispatch; inspect it again",
        ));
    }
    if action == "tools" {
        let catalog = match client.catalog(cancel) {
            Ok(catalog) => catalog,
            Err(error) => {
                *client = Client::new(connection.clone());
                return Err(error);
            }
        };
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
        return Ok(Value::object([
            ("connection", identity(&connection)),
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
        ]));
    }
    let name = text(arguments, "tool")?;
    let input = arguments
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| Value::object([]));
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
    if let Some(connections) = result.get("connections").and_then(Value::as_array) {
        return format!("{} configured connections", connections.len());
    }
    if let Some(tools) = result.get("tools").and_then(Value::as_array) {
        return format!("{} tools", tools.len());
    }
    if let Some(text) = content {
        return text.chars().take(240).collect();
    }
    "Completed".into()
}
