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
            "Read a project file or tmp:relative/path. Text is paged UTF-8; images and PDFs are recognized from their contents, copied into session storage and shown to you on the next request. The result includes an attachment:ID reference for rereading the copy after the original changes or disappears. Also accepts skill:<name> for a personal skill's SKILL.md or skill:<name>/<relative-path> for a text resource, saved output:SESSION:ID:stdout or :stderr, attachment:ID (text files page normally; images and PDFs are shown again; other files return their local path), or history:INDEX (a zero-based original message index in this session). history:requests returns the complete ordered user requests. history:memory returns the current conversation summary. checkpoint:rc-ID:path returns a file saved before one modifying call, until its fixed 24-hour expiry. History, skills, attachments and saved output remain available after compaction and resume. Older output:ID:stdout/:stderr references remain readable. Text paging uses one-based offset (default 1) and limit (default 200, maximum 2000). Pages contain up to 64 KiB; follow next_offset or next_byte_offset with byte_offset for a split long line. byte_offset is zero-based and returns unnumbered text; it takes precedence over offset. Paging arguments do not crop images or PDFs. A context_truncated tool result is an excerpt; read its history_reference for the full original, using small pages. total_lines is available at EOF in line mode.",
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
            "Create or replace a complete UTF-8 file. Project paths require an existing parent directory. Use tmp:relative/path for disposable scripts and one-off verification files; it writes in this session's temporary area and creates parent directories there. Keep project deliverables, including useful durable tests, in the project.",
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
            "Run a Bash command from the project directory. For recoverable writes, pass protect as workspace-relative file paths or recursive directory scopes; use a trailing slash for a new directory or '.' for the whole workspace. Recursive scopes skip generated and Git-ignored directories; name one explicitly to save it. Jecode saves their preimages before launch and reports saved, partial or none. For writes through a symlink, protect its target too. Only declared paths are covered; the command still runs if capture fails. stdin is closed and shell state does not carry over. timeout_seconds sets an execution deadline. JECODE_TMP, TMPDIR, TEMP and TMP point to session temporary storage; quote paths. Set explicit output paths under \"$JECODE_TMP\" for one-off verification artifacts; these variables do not redirect project-relative output. Without wait_ms the command runs in the foreground and returns exit_code, cancellation/timeout flags, stream byte counts, the last 64 KiB of stdout/stderr and read-only output references. With wait_ms it becomes a persistent process; Jecode waits up to wait_ms, then returns process_id and status. A running process continues until exit, timeout or stop. The checkpoint receipt describes only prelaunch coverage and expires 24 hours after capture. Bash has normal system access.",
            Value::object([
                ("command", string()),
                (
                    "protect",
                    Value::object([
                        ("type", Value::string("array")),
                        ("items", string()),
                        (
                            "description",
                            Value::string(
                                "Optional workspace-relative paths to save before Bash; existing directories are recursive, a trailing slash declares a new directory, and '.' covers the workspace.",
                            ),
                        ),
                    ]),
                ),
                ("timeout_seconds", integer(1, None)),
                ("wait_ms", integer(0, None)),
            ]),
            &["command"],
        ),
        tool(
            "process",
            "Manage persistent processes started by bash with wait_ms in this conversation. list returns every process_id with its command, status and output references; use it to recover IDs, for example after context compaction. poll returns the current status and the output saved since the previous bash or process result for that process, waiting up to wait_ms (default 0) for it to finish. stop requests termination of the process and its descendants and waits up to 10 seconds for the final status; if the wait expires or is interrupted it may return stopping, so poll to confirm completion. Each stream returns at most its last 64 KiB of new output: <stream>_offset is where the returned text starts in the saved stream, <stream>_next_offset is where the next result continues and <stream>_skipped_bytes counts earlier unread bytes left out; read <stream>_file with byte_offset to see them. exit_code appears only after the process exits. Empty output does not mean the process finished, is ready or is stalled; check status. Finished processes stay listed with their final status. Processes end when the conversation changes or Jecode exits and are never restarted; earlier IDs then return an error while saved output stays readable. wait_cancelled is true when the user interrupted the wait; it does not stop a running process or undo a previous stop request.",
            Value::object([
                (
                    "action",
                    Value::object([
                        ("type", Value::string("string")),
                        (
                            "enum",
                            Value::Array(["list", "poll", "stop"].map(Value::string).into()),
                        ),
                    ]),
                ),
                ("process_id", string()),
                ("wait_ms", integer(0, None)),
            ]),
            &["action"],
        ),
        tool(
            "checkpoint",
            "List, inspect and restore before-state snapshots from individual modifying calls. write and edit protect their file; Bash protects only declared paths. Each saved ref expires exactly 24 hours after capture. Inspect returns a token for the selected current paths. Restore requires that token or expected latest to acknowledge their current state, and saves an undo of paths it changes. A receipt reports what was saved before execution; it does not claim the tool succeeded.",
            Value::object([
                (
                    "action",
                    Value::object([
                        ("type", Value::string("string")),
                        (
                            "enum",
                            Value::Array(["list", "inspect", "restore"].map(Value::string).into()),
                        ),
                    ]),
                ),
                ("ref", string()),
                (
                    "paths",
                    Value::object([
                        ("type", Value::string("array")),
                        ("items", string()),
                        (
                            "description",
                            Value::string(
                                "Optional selected paths for inspect or restore. Omit to select the scope saved by the call.",
                            ),
                        ),
                    ]),
                ),
                (
                    "expected",
                    Value::object([
                        ("type", Value::string("string")),
                        (
                            "description",
                            Value::string(
                                "Required for restore: the fingerprint from inspect for these paths, or latest to acknowledge the current live state.",
                            ),
                        ),
                    ]),
                ),
                (
                    "preview",
                    Value::object([("type", Value::string("boolean"))]),
                ),
                ("limit", integer(1, Some(200))),
            ]),
            &["action"],
        ),
    ])
}

#[cfg(test)]
mod tests;
