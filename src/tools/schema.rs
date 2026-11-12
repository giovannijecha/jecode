use crate::json::Value;
use std::sync::OnceLock;
mod collaboration;
mod lsp;
mod mcp;
mod web;
mod work;

/// Executable tools in Jecode's neutral name, description and parameters shape.
/// Each backend owns the wrapper required by its request protocol.
pub fn definitions() -> &'static Value {
    static DEFINITIONS: OnceLock<Value> = OnceLock::new();
    DEFINITIONS.get_or_init(build_definitions)
}

pub fn encoded_size() -> usize {
    static SIZE: OnceLock<usize> = OnceLock::new();
    *SIZE.get_or_init(|| definitions().encode().len())
}

pub fn definitions_for(delegation: bool) -> &'static Value {
    static WORKING: OnceLock<Value> = OnceLock::new();
    if delegation {
        definitions()
    } else {
        WORKING.get_or_init(|| {
            let mut tools: Vec<_> = definitions()
                .as_array()
                .unwrap()
                .iter()
                .filter(|tool| {
                    !matches!(
                        tool.get("name").and_then(Value::as_str),
                        Some("agent" | "ask_user")
                    )
                })
                .cloned()
                .collect();
            let index = tools
                .iter()
                .position(|tool| tool.get("name").and_then(Value::as_str) == Some("steps"))
                .unwrap();
            tools[index] = work::steps(&tools);
            Value::Array(tools)
        })
    }
}

pub fn encoded_size_for(delegation: bool) -> usize {
    static WORKING_SIZE: OnceLock<usize> = OnceLock::new();
    if delegation {
        encoded_size()
    } else {
        *WORKING_SIZE.get_or_init(|| definitions_for(false).encode().len())
    }
}

fn string() -> Value {
    Value::object([("type", Value::string("string"))])
}

fn integer(min: usize, max: Option<usize>) -> Value {
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
}

fn boolean() -> Value {
    Value::object([("type", Value::string("boolean"))])
}

fn strings() -> Value {
    Value::object([("type", Value::string("array")), ("items", string())])
}

/// An edit's ordered replacements.
fn replacements() -> Value {
    Value::object([
        ("type", Value::string("array")),
        ("minItems", Value::number(1)),
        (
            "items",
            Value::object([
                ("type", Value::string("object")),
                (
                    "properties",
                    Value::object([("old_text", string()), ("new_text", string())]),
                ),
                (
                    "required",
                    Value::Array(["old_text", "new_text"].map(Value::string).into()),
                ),
                ("additionalProperties", Value::Bool(false)),
            ]),
        ),
    ])
}

fn build_definitions() -> Value {
    let mut tools = base_definitions();
    tools.extend(super::discovery::definitions());
    tools.push(mcp::definition());
    tools.push(lsp::definition());
    tools.push(collaboration::definition());
    let steps_index = tools.len();
    tools.push(work::todo());
    tools.push(work::ask_user());
    let steps = work::steps(&tools);
    tools.insert(steps_index, steps);
    Value::Array(tools)
}

pub(super) fn tool(
    name: &str,
    description: &str,
    mut properties: Value,
    required: &[&str],
) -> Value {
    if let Value::Object(fields) = &mut properties {
        fields.insert(
            "parallel".into(),
            Value::object([
                ("type", Value::string("boolean")),
                (
                    "description",
                    Value::string(
                        "True declares independence from adjacent parallel calls (up to four); false requires serial execution.",
                    ),
                ),
            ]),
        );
    }
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
                    Value::Array(required.iter().map(|name| Value::string(*name)).collect()),
                ),
                ("additionalProperties", Value::Bool(false)),
            ]),
        ),
    ])
}

/// The optional label that groups related calls into one Work chapter.
pub(super) fn work() -> Value {
    Value::object([
        ("type", Value::string("string")),
        (
            "description",
            Value::string("Optional short work label. Repeat it exactly for related calls."),
        ),
    ])
}

pub(crate) fn routine(
    name: &str,
    description: &str,
    mut properties: Value,
    required: &[&str],
) -> Value {
    let Value::Object(fields) = &mut properties else {
        unreachable!()
    };
    fields.insert("work".into(), work());
    fields.insert(
            "description".into(),
            Value::object([
                ("type", Value::string("string")),
                (
                    "description",
                    Value::string(
                        "Optional short description of this call for the user. Omit it when the command is clear enough.",
                    ),
                ),
            ]),
        );
    tool(name, description, properties, required)
}

fn base_definitions() -> Vec<Value> {
    vec![
        routine(
            "read",
            "Read a project file or tmp:relative/path, or several at once with paths: when a step needs more than one file, read them in one call (each with the default window, results in order; outside paths need the same permission as single reads, one approval per call; media need a single path; results past the inline size are saved in order to result_file, read it with offset and limit). UTF-8 text is paged; images/PDFs are detected by content, copied, and shown next request. Their attachment:ID survives source changes. Other paths: skill:<name>[/relative-path] (SKILL.md by default); output:SESSION:ID:stdout/:stderr (older output:ID refs also work); attachment:ID (text pages, media shown again, other files return a path); history:INDEX (zero-based original message), history:requests (ordered user requests), history:memory (conversation summary), history:processes (saved process outcomes). Input withdrawn to the composer is excluded from history reads. History, skills, attachments and output survive compaction/resume. Text pages default to offset 1 and limit 200 (at most 2000 returned per page), capped at 64 KiB. Follow next_offset, or next_byte_offset with byte_offset for split long lines. byte_offset takes precedence and returns unnumbered text. Paging arguments do not crop media. For context_truncated results, read history_reference in small pages. total_lines appears at EOF in line mode.",
            Value::object([
                ("path", string()),
                (
                    "paths",
                    Value::object([
                        ("type", Value::string("array")),
                        ("minItems", Value::number(1)),
                        ("items", string()),
                    ]),
                ),
                ("offset", integer(1, None)),
                ("limit", integer(1, None)),
                ("byte_offset", integer(0, None)),
            ]),
            &[],
        ),
        tool(
            "write",
            "Create or replace a complete UTF-8 file, creating any missing parent directories; text in the assistant reply does not execute a write. tmp:relative/path writes to session temporary storage for disposable scripts and one-off verification. Keep deliverables and durable tests in the project.",
            Value::object([("path", string()), ("content", string())]),
            &["path", "content"],
        ),
        tool(
            "edit",
            "Edit a UTF-8 project or tmp: file with path and one or more ordered replacements. Each nonempty old_text must match exactly once after preceding edits; any missing or ambiguous match leaves the file unchanged. Preserve exact whitespace and line endings. For several files send files ({path, edits} per file) instead of path and edits: each file applies all or nothing, the others are still written, and results follow call order; retry only entries with an error.",
            Value::object([
                ("path", string()),
                ("edits", replacements()),
                (
                    "files",
                    Value::object([
                        ("type", Value::string("array")),
                        ("minItems", Value::number(1)),
                        (
                            "items",
                            Value::object([
                                ("type", Value::string("object")),
                                (
                                    "properties",
                                    Value::object([("path", string()), ("edits", replacements())]),
                                ),
                                (
                                    "required",
                                    Value::Array(["path", "edits"].map(Value::string).into()),
                                ),
                                ("additionalProperties", Value::Bool(false)),
                            ]),
                        ),
                    ]),
                ),
            ]),
            &[],
        ),
        routine(
            "bash",
            "Run Jecode's built-in command language in the project directory on every host. It supports quotes, $variables, ${variable:-default}, ;, newlines, &&, ||, pipelines, file redirection including 2>&1 and &>, here-documents, * ? [...] patterns, the built-ins cat, cd, cp, echo, env, export, grep, head, ls, mkdir, mktemp, mv, printenv, printf, pwd, rm, seq, sleep, tail, test, touch, tr -d, wc -l/-c, and literal argv for installed programs. It is not GNU Bash; unsupported syntax fails explicitly. stdin is closed; shell state does not persist. JECODE_TMP, TMPDIR, TEMP and TMP point to session temporary storage; quote paths and write disposable output under \"$JECODE_TMP\". Run related commands together in one command line instead of one call each. Every command is a managed process. Without wait_ms the call waits up to 10 seconds: a command that ends in time returns exit/cancellation/timeout status, byte counts, 16 KiB head/tail previews per stream and full saved-output refs; interrupting that wait stops the command. Still running, it returns yielded:true, a process_id and its next output window, and keeps running. Set wait_ms to choose the wait; it returns the process_id form, and interrupting it leaves the command running. Waiting never ends execution; timeout_seconds does, and omitted means no deadline. The outcome reaches you automatically when the process ends, so continue independent work; builds, tests, generation and exports still need their outcome before dependent work or a claim of success. Servers and watchers need a real readiness check; a live process or empty output does not establish readiness. output_limit_bytes sets a positive per-stream budget, capped at 256 KiB; logs remain complete. poll/stop via process. Shell & does not create a separate process_id.",
            Value::object([
                ("command", string()),
                (
                    "timeout_seconds",
                    Value::object([
                        ("type", Value::string("integer")),
                        ("minimum", Value::number(1)),
                        (
                            "description",
                            Value::string(
                                "Optional execution deadline in seconds from launch; omitted means no deadline. Independent of wait_ms.",
                            ),
                        ),
                    ]),
                ),
                (
                    "wait_ms",
                    Value::object([
                        ("type", Value::string("integer")),
                        ("minimum", Value::number(0)),
                        (
                            "description",
                            Value::string(
                                "Initial wait in milliseconds before returning a process_id; does not end the process. Omit for the 10-second default, which returns the final result when the command ends in time.",
                            ),
                        ),
                    ]),
                ),
                ("output_limit_bytes", integer(1, None)),
            ]),
            &["command"],
        ),
        routine(
            "process",
            "Manage conversation-owned Bash and exec processes, active and recently finished. list recovers IDs, status, elapsed time, commands and output refs. poll waits up to wait_ms (default 0) and returns the next unread output window; output_mode:tail jumps to the newest unread bytes and reports skipped bytes, still readable from the saved file. A finished process is reported to you once at your next request, so poll to read output or to wait with nothing else to do, not to learn that it ended. moved_to_background means the user released that wait; the process keeps running. stop terminates the process and descendants; poll if it returns stopping. For tty exec, write sends text or one key (see key enum), resize changes cols/rows and interrupt sends Ctrl+C. Text writes accept any length and send confirmed segments in order; after unconfirmed delivery, later bytes are not sent and nothing is retried. A write receipt confirms control-pipe delivery, not command completion; unconfirmed input may still arrive, so inspect before retrying. Output windows default to 16 KiB per stream; output_limit_bytes is capped at 256 KiB. Poll until drained or read the saved file with byte_offset. Empty output does not prove readiness; check status. exit_code appears after exit. wait_cancelled interrupts only the wait. Processes end on conversation change or Jecode exit; saved output remains readable.",
            Value::object([
                (
                    "action",
                    Value::object([
                        ("type", Value::string("string")),
                        (
                            "enum",
                            Value::Array(
                                ["list", "poll", "stop", "write", "resize", "interrupt"]
                                    .map(Value::string)
                                    .into(),
                            ),
                        ),
                    ]),
                ),
                ("process_id", string()),
                ("text", string()),
                (
                    "key",
                    Value::object([
                        ("type", Value::string("string")),
                        (
                            "enum",
                            Value::Array(
                                [
                                    "Enter",
                                    "Tab",
                                    "Escape",
                                    "Backspace",
                                    "Up",
                                    "Down",
                                    "Left",
                                    "Right",
                                    "Home",
                                    "End",
                                    "Delete",
                                    "Ctrl+C",
                                    "Ctrl+D",
                                    "Ctrl+Z",
                                ]
                                .map(Value::string)
                                .into(),
                            ),
                        ),
                    ]),
                ),
                ("cols", integer(1, Some(1000))),
                ("rows", integer(1, Some(1000))),
                ("wait_ms", integer(0, None)),
                ("output_limit_bytes", integer(1, None)),
                (
                    "output_mode",
                    Value::object([
                        ("type", Value::string("string")),
                        (
                            "enum",
                            Value::Array(["next", "tail"].map(Value::string).into()),
                        ),
                    ]),
                ),
            ]),
            &["action"],
        ),
        web::web(),
        web::browser(),
        routine(
            "exec",
            "Run one executable with literal argv and no implicit shell; for several commands, use one bash call. A bare name uses PATH, including any shim found there; Jecode does not bypass it. cwd defaults to the project directory; relative cwd uses the project root, and executable paths with a directory use cwd. env strings set variables and null removes them after inherited provider credentials are stripped. stdin sends a string then closes; omitted means EOF. tty:true rejects stdin and always returns a process_id; use process write/resize/interrupt. Terminal stdout combines child streams; stderr holds bridge diagnostics. Without tty, waiting, yielding, outcome delivery, previews and saved refs follow Bash: omitted wait_ms waits up to 10 seconds for a final result, then returns yielded:true and a process_id while the program keeps running. timeout_seconds bounds execution.",
            Value::object([
                ("executable", string()),
                ("args", strings()),
                ("cwd", string()),
                (
                    "env",
                    Value::object([
                        ("type", Value::string("object")),
                        (
                            "additionalProperties",
                            Value::object([(
                                "type",
                                Value::Array(
                                    [Value::string("string"), Value::string("null")].into(),
                                ),
                            )]),
                        ),
                    ]),
                ),
                ("stdin", string()),
                ("tty", boolean()),
                ("cols", integer(1, Some(1000))),
                ("rows", integer(1, Some(1000))),
                ("wait_ms", integer(0, None)),
                ("timeout_seconds", integer(1, None)),
                ("output_limit_bytes", integer(1, None)),
            ]),
            &["executable"],
        ),
    ]
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod web_tests;
