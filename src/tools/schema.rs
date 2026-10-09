use crate::json::Value;

pub fn definitions() -> Value {
    let string = || Value::object([("type", Value::string("string"))]);
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
            "Read a UTF-8 project file, tmp:relative/path, saved output:SESSION:ID:stdout or :stderr, attachment:ID for a file the user attached (text files page like project files; images and PDFs are shown to you again; other files return their local path), or history:INDEX (a zero-based original message index in this session). history:requests returns the complete ordered user requests. history:memory returns the current conversation summary. History, attachments and saved output are read-only and remain available after compaction and resume. Older output:ID:stdout/:stderr references remain readable. offset is one-based (default 1), limit defaults to 200 (maximum 2000). Pages contain up to 64 KiB; follow next_offset or next_byte_offset with byte_offset for a split long line. byte_offset is zero-based and returns unnumbered text; it takes precedence over offset. A context_truncated tool result is an excerpt; read its history_reference for the full original, using small pages. total_lines is available at EOF in line mode.",
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
            "bash",
            "Run a foreground Bash command from the project working directory. stdin is closed. No timeout unless positive timeout_seconds is supplied. JECODE_TMP, TMPDIR, TEMP and TMP point to persistent session temporary storage; quote paths. Returns exit_code, cancellation/timeout flags, stream byte counts, the last 64 KiB of stdout/stderr and read-only output references; full streams remain readable through them. Bash has normal system access; use finite foreground commands.",
            Value::object([
                ("command", string()),
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
