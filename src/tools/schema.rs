use crate::json::Value;

pub fn definitions() -> Value {
    let string = || Value::object([("type", Value::string("string"))]);
    let described = |mut value: Value, description: &str| {
        if let Value::Object(fields) = &mut value {
            fields.insert("description".into(), Value::string(description));
        }
        value
    };
    let integer = |min, max: Option<usize>| {
        let mut value = Value::object([
            ("type", Value::string("integer")),
            ("minimum", Value::number(min)),
        ]);
        if let Some(max) = max
            && let Value::Object(fields) = &mut value
        {
            fields.insert("maximum".into(), Value::number(max));
        }
        value
    };
    let tool = |name, description, properties, required: &[&str]| {
        Value::object([
            ("type", Value::string("function")),
            (
                "function",
                Value::object([
                    ("name", Value::string(name)),
                    ("description", Value::string(description)),
                    (
                        "parameters",
                        Value::object([
                            ("type", Value::string("object")),
                            ("properties", properties),
                            (
                                "required",
                                Value::Array(
                                    required.iter().map(|name| Value::string(*name)).collect(),
                                ),
                            ),
                            ("additionalProperties", Value::Bool(false)),
                        ]),
                    ),
                ]),
            ),
        ])
    };
    Value::Array(vec![
        tool(
            "read",
            "Read a UTF-8 project file, tmp:relative/path, saved output:SESSION:ID:stdout or :stderr, or history:INDEX (a zero-based original message index in this session). history:requests returns the complete ordered user requests. history:memory returns the complete current operational ledger, including completed work archived from the active view. History and saved output are read-only and remain available after compaction and resume. Older output:ID:stdout/:stderr references remain readable. offset is one-based (default 1), limit defaults to 200 (maximum 2000). Pages contain up to 64 KiB; follow next_offset or next_byte_offset with byte_offset for a split long line. byte_offset is zero-based and returns unnumbered text; it takes precedence over offset. A context_truncated tool result is an excerpt; read its history_reference for the full original, using small pages. total_lines is available at EOF in line mode.",
            Value::object([
                ("path", string()),
                ("offset", integer(1, None)),
                ("limit", integer(1, Some(2000))),
                ("byte_offset", integer(0, None)),
            ]),
            &["path"],
        ),
        tool(
            "write",
            "Create or replace a complete UTF-8 file. Project paths require an existing parent directory. tmp:relative/path writes disposable working files in this session's temporary area and creates parent directories there. Keep durable source code, tests and build recipes in the project.",
            Value::object([("path", string()), ("content", string())]),
            &["path", "content"],
        ),
        tool(
            "edit",
            "Replace one exact, nonempty old_text occurrence in a UTF-8 project file or tmp:relative/path in this session's temporary area with new_text. Missing or ambiguous matches fail without changing the file. Include original whitespace and line endings.",
            Value::object([
                ("path", string()),
                ("old_text", string()),
                ("new_text", string()),
            ]),
            &["path", "old_text", "new_text"],
        ),
        tool(
            "protect",
            "Preserve existing files/directories only when the user explicitly requires their bytes to remain unchanged. A request to edit or delete a file permits that change: do not record that target. record is not a backup step or a prerequisite for ordinary file operations. Use write/edit/bash directly when no carried protection blocks the operation. Preserve behavior/API compatibility through editable source, not byte-for-byte registration. Record required immutable files before mutations; directories snapshot current files and retain their declared scope. New files stay editable in their creating request; later requests must record existing covered_by_recorded_directory candidates before project mutations or Bash. Status exclusions cannot waive directory declarations or active baselines. Other candidates need record or status with scope_exclusions/reason. record requires paths/reason; require_check:true demands a passed check after the last tracked mutation. Active baselines cannot be rebased. status returns current tokens and bounded inventory; explain incomplete scope review with reason. not_started means no execution. restore takes one path/latest expected_current, restoring exact binary bytes without retyping; newer changes, unavailable baselines and read-only targets are refused. release requires one path/reason and a newer user request explicitly permitting edits or a scope change. Releasing a directory declaration leaves individual baselines active. Interrupted registration remains unknown and blocks mutations; release its status marker only after a newer decision accepting the unavailable original. Protections and directory/scope decisions survive compaction/resume. Violated/unknown baselines or unresolved scope block completion. Bash has normal access. Natural-language interpretation still depends on the model.",
            Value::object([
                (
                    "action",
                    Value::object([
                        ("type", Value::string("string")),
                        (
                            "enum",
                            Value::Array(
                                ["record", "status", "restore", "release"]
                                    .iter()
                                    .map(|value| Value::string(*value))
                                    .collect(),
                            ),
                        ),
                    ]),
                ),
                (
                    "paths",
                    Value::object([("type", Value::string("array")), ("items", string())]),
                ),
                (
                    "reason",
                    described(
                        string(),
                        "record: cite the user's explicit byte-preservation requirement. release: explain the newer user decision allowing changes. status: explain an actual scope decision; an empty value makes no decision.",
                    ),
                ),
                (
                    "scope_exclusions",
                    described(
                        Value::object([("type", Value::string("array")), ("items", string())]),
                        "Nonempty exclusions belong only to action=status and require reason. Omit for other actions; an empty array excludes nothing.",
                    ),
                ),
                (
                    "expected_current",
                    described(
                        string(),
                        "Only action=restore uses the latest status token; omit for other actions.",
                    ),
                ),
                (
                    "require_check",
                    Value::object([("type", Value::string("boolean"))]),
                ),
            ]),
            &["action"],
        ),
        tool(
            "bash",
            "Run a foreground Bash command from the project working directory. Archived exact commands need repeat_reason after reviewing the previous result and user request. Bash defaults to errexit and pipefail so unhandled pipeline failures are visible. Use check:true for build/test/verification commands to record check_status as well. Run separate checks separately; capture expected nonzero exits explicitly in negative tests. Use explicit check:false only when ordinary shell control flow is needed; successful tail/echo can then mask earlier failure. Optional watch is an array of existing project file paths to observe before execution, useful for protected files inspected through Bash. Files previously read/written/edited are observed automatically. Results include file_changes with before/after fingerprints, source interval, first_observed and history references; inspect unintended changes, preserve constraints and recheck after mutations. file_tracking reports the limited scope and incomplete comparisons; an empty change list does not cover unobserved files. Results include shell_mode. JECODE_TMP, TMPDIR, TEMP and TMP point to persistent session temporary storage; quote paths. stdin is closed. No timeout unless positive timeout_seconds is supplied. Returns exit_code, check/check_status, cancellation/timeout flags, stream byte counts, the last 64 KiB of stdout/stderr and read-only output references. Full streams remain readable. Bash has normal system access; use finite foreground commands.",
            Value::object([
                ("command", string()),
                ("check", Value::object([("type", Value::string("boolean"))])),
                (
                    "repeat_reason",
                    described(
                        string(),
                        "A nonempty explanation authorizes a deliberate rerun of an archived exact command after reviewing its result. Omit or leave empty for ordinary first executions; an empty value grants no repeat permission.",
                    ),
                ),
                (
                    "watch",
                    Value::object([("type", Value::string("array")), ("items", string())]),
                ),
                (
                    "timeout_seconds",
                    Value::object([
                        ("type", Value::string("integer")),
                        ("minimum", Value::number(1)),
                    ]),
                ),
            ]),
            &["command"],
        ),
    ])
}

#[cfg(test)]
mod tests;
