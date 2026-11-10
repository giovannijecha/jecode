use super::{routine, string};
use crate::json::Value;

pub(super) fn definition() -> Value {
    routine(
        "mcp",
        "Use globally configured MCP connections. list shows configured connections only; tools discovers exact remote tool names and input schemas (query/offset/limit page them); call forwards the remote tool and arguments unchanged. Use a connection's stable id from list. Remote permissions come from the enabled connection and its service, independently of local Read/Workspace/Full. Jecode cannot undo remote actions. If an outcome is unknown, inspect the remote state before repeating the operation.",
        Value::object([
            (
                "action",
                Value::object([
                    ("type", Value::string("string")),
                    (
                        "enum",
                        Value::Array(["list", "tools", "call"].map(Value::string).to_vec()),
                    ),
                ]),
            ),
            ("connection", string()),
            ("tool", string()),
            (
                "arguments",
                Value::object([
                    ("type", Value::string("object")),
                    ("additionalProperties", Value::Bool(true)),
                ]),
            ),
            ("query", string()),
            ("offset", super::integer(0, None)),
            ("limit", super::integer(1, Some(50))),
        ]),
        &["action"],
    )
}
