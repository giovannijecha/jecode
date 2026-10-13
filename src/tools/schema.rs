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
            "Run a Bash command in a new process from the project working directory. stdin is closed and no shell state carries over between commands. No timeout unless positive timeout_seconds is supplied; it is an execution deadline from command start. JECODE_TMP, TMPDIR, TEMP and TMP point to persistent session temporary storage; quote paths. Set explicit output paths under \"$JECODE_TMP\" for one-off verification artifacts; these variables do not redirect project-relative output. Without wait_ms the command runs in the foreground and returns exit_code, cancellation/timeout flags, stream byte counts, the last 64 KiB of stdout/stderr and read-only output references; full streams remain readable through them. With wait_ms (0 or more) the command becomes a persistent process: Jecode waits up to wait_ms for it to finish, then returns process_id and status (running, exited with exit_code, timed_out, stopped or unknown) with output as the process tool reports it. A running process keeps running after the wait, across tool calls and turns, until it exits, reaches timeout_seconds or is stopped; use it for servers, watchers and long builds or tests. Bash has normal system access.",
            Value::object([
                ("command", string()),
                (
                    "timeout_seconds",
                    Value::object([
                        ("type", Value::string("integer")),
                        ("minimum", Value::number(1)),
                    ]),
                ),
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
    ])
}

#[cfg(test)]
mod tests;
