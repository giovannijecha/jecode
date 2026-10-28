use super::ToolCall;
use crate::json::{self, Value};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Mode {
    Serial,
    Automatic,
    Declared,
}

pub(super) fn mode(call: &ToolCall) -> Mode {
    if matches!(call.name.as_str(), "agent" | "todo_list" | "ask_user") {
        return Mode::Serial;
    }
    let Ok(arguments) = json::parse(&call.arguments) else {
        return Mode::Serial;
    };
    // Agent-owned references depend on transcript state or a per-turn cache.
    if call.name == "read"
        && arguments
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|path| path.starts_with("history:") || path.starts_with("skill:"))
    {
        return Mode::Serial;
    }
    match crate::tools::parallel(&arguments) {
        Ok(Some(true)) => Mode::Declared,
        Ok(Some(false)) | Err(_) => Mode::Serial,
        Ok(None) if automatic(call, &arguments) => Mode::Automatic,
        Ok(None) => Mode::Serial,
    }
}

fn automatic(call: &ToolCall, arguments: &Value) -> bool {
    let action = arguments.get("action").and_then(Value::as_str);
    match call.name.as_str() {
        "list" | "search" => true,
        "map" => matches!(action, None | Some("overview")),
        "read" => arguments.get("path").and_then(Value::as_str).is_some(),
        "checkpoint" => matches!(action, Some("list" | "inspect")),
        "process" => action == Some("list"),
        "web" => {
            let method = arguments
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("GET");
            matches!(action, Some("fetch" | "download"))
                && matches!(method, "GET" | "HEAD")
                && arguments.get("body").is_none()
        }
        _ => false,
    }
}

#[cfg(test)]
pub(super) fn parallel_call(call: &ToolCall) -> bool {
    mode(call) != Mode::Serial
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, parallel: Option<Value>, path: &str) -> ToolCall {
        let mut args = Value::object([("path", Value::string(path))]);
        if let Some(value) = parallel
            && let Value::Object(fields) = &mut args
        {
            fields.insert("parallel".into(), value);
        }
        ToolCall {
            id: "one".into(),
            name: name.into(),
            arguments: args.encode(),
        }
    }

    #[test]
    fn declarations_do_not_merge_with_automatic_reads_or_override_serial_references() {
        assert_eq!(mode(&call("write", None, "a")), Mode::Serial);
        assert_eq!(
            mode(&call("write", Some(Value::Bool(true)), "a")),
            Mode::Declared
        );
        assert_eq!(mode(&call("read", None, "a")), Mode::Automatic);
        assert_eq!(
            mode(&call("read", Some(Value::Bool(true)), "a")),
            Mode::Declared
        );
        assert_eq!(
            mode(&call("read", Some(Value::Bool(false)), "a")),
            Mode::Serial
        );
        assert_eq!(
            mode(&call("read", Some(Value::string("true")), "a")),
            Mode::Serial
        );
        for path in ["history:requests", "skill:example"] {
            assert_eq!(
                mode(&call("read", Some(Value::Bool(true)), path)),
                Mode::Serial
            );
        }
    }
}
