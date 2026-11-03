use super::{boolean, integer, routine, string};
use crate::json::Value;

pub(super) fn definition() -> Value {
    routine(
        "lsp",
        "Query user-enabled language servers. status lists configured servers and observed conversation instances without starting anything. definition and references use path plus one-based line and Unicode-scalar column. diagnostics covers the requested document only: inspect freshness, completeness and coverage; pending/stale/unverified reports never establish a clean project. Use a stable server ID from status to resolve ambiguous matches. Servers start lazily and are reused for this conversation/root. Enablement authorizes the selected external server independently of local Read/Workspace/Full; this tool cannot configure servers or execute workspace edits. Large results retain result_file; read it for omitted locations or diagnostics.",
        Value::object([
            (
                "action",
                Value::object([
                    ("type", Value::string("string")),
                    (
                        "enum",
                        Value::Array(
                            ["status", "definition", "references", "diagnostics"]
                                .map(Value::string)
                                .to_vec(),
                        ),
                    ),
                ]),
            ),
            ("server", string()),
            ("path", string()),
            ("line", integer(1, Some(i32::MAX as usize))),
            ("column", integer(1, Some(i32::MAX as usize))),
            ("include_declaration", boolean()),
        ]),
        &["action"],
    )
}
