use crate::json::Value;

pub fn description(name: &str, arguments: &Value) -> String {
    if matches!(
        name,
        "read" | "list" | "search" | "map" | "bash" | "exec" | "process" | "web" | "browser"
    ) && let Some(label) = arguments.get("description").and_then(Value::as_str)
        && !label.trim().is_empty()
    {
        return label.trim().into();
    }
    target(name, arguments)
}

/// The invocation target, independently of an optional display description.
pub fn target(name: &str, arguments: &Value) -> String {
    if name == "agent" {
        let action = arguments
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("invalid action");
        if action == "create" {
            let count = arguments
                .get("tasks")
                .and_then(Value::as_array)
                .map_or(0, <[Value]>::len);
            return format!(
                "create {count} {}",
                if count == 1 {
                    "assignment"
                } else {
                    "assignments"
                }
            );
        }
        let id = arguments.get("id").and_then(Value::as_str).unwrap_or("");
        let label = id
            .strip_prefix('a')
            .filter(|suffix| !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit()))
            .map_or_else(|| id.to_owned(), |number| format!("AGENT {number}"));
        return format!("{action} {label}").trim_end().into();
    }
    if name == "map" {
        let field = |key| arguments.get(key).and_then(Value::as_str).unwrap_or("");
        return match field("action") {
            "remember" => field("title").to_owned(),
            "forget" => field("id").to_owned(),
            _ => {
                let path = if field("path").is_empty() {
                    "."
                } else {
                    field("path")
                };
                match field("query") {
                    "" => path.to_owned(),
                    query => format!("{query} in {path}"),
                }
            }
        };
    }
    if name == "list" {
        return arguments
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .into();
    }
    if name == "search" {
        let query = arguments
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("invalid arguments");
        let path = arguments.get("path").and_then(Value::as_str).unwrap_or(".");
        return format!("{query} in {path}");
    }
    if name == "web" || name == "browser" {
        let field = |key| arguments.get(key).and_then(Value::as_str).unwrap_or("");
        let action = field("action");
        let target = if name == "web" {
            if action == "crawl" {
                let urls = arguments
                    .get("urls")
                    .and_then(Value::as_array)
                    .unwrap_or(&[]);
                if let Some(first) = urls.first().and_then(Value::as_str) {
                    return if urls.len() == 1 {
                        format!("crawl {first}")
                    } else {
                        format!("crawl {first} (+{} URLs)", urls.len() - 1)
                    };
                }
                field("crawl_id")
            } else {
                field("url")
            }
        } else {
            match action {
                "open" | "navigate" => field("url"),
                "click" | "fill" | "wait" => field("selector"),
                "press" => field("key"),
                "evaluate" => field("expression"),
                "cdp" => field("method"),
                _ => field("tab_id"),
            }
        };
        return format!("{action} {target}").trim_end().into();
    }
    if name == "process" {
        let field = |key| arguments.get(key).and_then(Value::as_str).unwrap_or("");
        return format!("{} {}", field("action"), field("process_id"))
            .trim_end()
            .into();
    }
    if name == "checkpoint" {
        let field = |key| arguments.get(key).and_then(Value::as_str).unwrap_or("");
        let mut words = vec![field("action"), field("ref")];
        if let Some(paths) = arguments.get("paths").and_then(Value::as_array) {
            words.extend(paths.iter().filter_map(Value::as_str));
        }
        words.retain(|word| !word.is_empty());
        return words.join(" ");
    }
    if name == "bash" {
        return arguments
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("invalid arguments")
            .into();
    }
    if name == "exec" {
        let executable = arguments
            .get("executable")
            .and_then(Value::as_str)
            .unwrap_or("invalid arguments");
        return match arguments.get("args").and_then(Value::as_array) {
            Some(args) if !args.is_empty() => {
                format!("{executable} {}", Value::Array(args.to_vec()).encode())
            }
            _ => executable.into(),
        };
    }
    let path = arguments
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("invalid arguments");
    if name == "read" {
        let offset = arguments
            .get("offset")
            .and_then(Value::as_usize)
            .unwrap_or(1);
        let limit = arguments
            .get("limit")
            .and_then(Value::as_usize)
            .unwrap_or(200);
        return format!("{path} (from line {offset}, up to {limit} lines)");
    }
    path.into()
}
