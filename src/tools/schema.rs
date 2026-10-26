use crate::json::Value;
use std::sync::OnceLock;

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

fn build_definitions() -> Value {
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
    let boolean = || Value::object([("type", Value::string("boolean"))]);
    let strings = || Value::object([("type", Value::string("array")), ("items", string())]);
    let tool = |name, description, properties, required: &[&str]| {
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
    };
    let routine = |name, description, mut properties: Value, required: &[&str]| {
        let Value::Object(fields) = &mut properties else {
            unreachable!()
        };
        fields.insert(
            "work".into(),
            Value::object([
                ("type", Value::string("string")),
                (
                    "description",
                    Value::string(
                        "Optional short work label. Repeat it exactly for related calls.",
                    ),
                ),
            ]),
        );
        tool(name, description, properties, required)
    };
    Value::Array(vec![
        routine(
            "read",
            "Read a project file or tmp:relative/path. UTF-8 text is paged; images/PDFs are detected by content, copied, and shown next request. Their attachment:ID survives source changes. Other paths: skill:<name>[/relative-path] (SKILL.md by default); output:SESSION:ID:stdout/:stderr (older output:ID refs also work); attachment:ID (text pages, media shown again, other files return a path); history:INDEX (zero-based original message), history:requests (ordered user requests), history:memory (conversation summary). Input withdrawn to the composer is excluded from history reads. checkpoint:rc-ID:path reads a saved preimage (expires after 24 hours). History, skills, attachments and output survive compaction/resume. Text pages default to offset 1 and limit 200 (max 2000), capped at 64 KiB. Follow next_offset, or next_byte_offset with byte_offset for split long lines. byte_offset takes precedence and returns unnumbered text. Paging arguments do not crop media. For context_truncated results, read history_reference in small pages. total_lines appears at EOF in line mode.",
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
            "Create or replace a complete UTF-8 file; text in the assistant reply does not execute a write. Project paths need an existing parent. tmp:relative/path creates parents in session temporary storage for disposable scripts and one-off verification. Keep deliverables and durable tests in the project.",
            Value::object([("path", string()), ("content", string())]),
            &["path", "content"],
        ),
        tool(
            "edit",
            "Edit a UTF-8 project or tmp: file with one or more ordered replacements. Each nonempty old_text must match exactly once after preceding edits; any missing or ambiguous match leaves the file unchanged. Preserve exact whitespace and line endings.",
            Value::object([
                ("path", string()),
                (
                    "edits",
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
                                    Value::Array(
                                        ["old_text", "new_text"].map(Value::string).into(),
                                    ),
                                ),
                                ("additionalProperties", Value::Bool(false)),
                            ]),
                        ),
                    ]),
                ),
            ]),
            &["path", "edits"],
        ),
        routine(
            "bash",
            "Run Bash in the project directory. Shell syntax is Bash on every host, including Windows; invoke PowerShell explicitly if needed. Use protect for recoverable writes: workspace-relative files or recursive directories, new directories with trailing '/', or '.' for the workspace. Generated/ignored directories need explicit names; protect a symlink target too. Only declared paths are saved before launch; capture failure does not prevent execution. stdin is closed; shell state does not persist. JECODE_TMP, TMPDIR, TEMP and TMP point to session temporary storage; quote paths and write disposable output under \"$JECODE_TMP\" (project-relative paths are unchanged). Without wait_ms, returns exit/cancellation/timeout status, byte counts, 16 KiB head/tail previews per stream and full saved-output refs. output_limit_bytes sets a positive per-stream budget, capped at 256 KiB; larger requests are clamped and logs remain complete. With wait_ms, returns a persistent process_id and its next output window; poll/stop via process. wait_ms does not end execution; timeout_seconds does. Checkpoint receipts describe prelaunch coverage, not success. Bash has normal system access.",
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
                                "Initial wait in milliseconds before returning a persistent process_id; does not end the process. Omit to wait for command completion.",
                            ),
                        ),
                    ]),
                ),
                (
                    "output_limit_bytes",
                    integer(1, Some(super::processes::MAX_OUTPUT_LIMIT)),
                ),
            ]),
            &["command"],
        ),
        routine(
            "process",
            "Manage this conversation's persistent Bash processes. list recovers IDs, status, commands and saved-output refs. poll waits up to wait_ms (default 0) and returns status plus the next unread window. output_mode defaults to next; tail jumps to the newest unread bytes for readiness/status checks and reports skipped bytes, still readable from the saved file. stop requests termination of the process and descendants, waits up to 10 seconds, and may return stopping; poll to confirm. Windows are 16 KiB per stream by default; output_limit_bytes sets a positive budget capped at 256 KiB; larger requests are clamped. <stream>_offset and <stream>_next_offset track consecutive saved bytes; _remaining_bytes counts unread saved data; _skipped_bytes is zero in next mode. Poll until drained, or read <stream>_file with byte_offset. Empty output says nothing about readiness; check status. exit_code appears after exit; finished processes remain listed. wait_cancelled interrupts the wait, not the process or a prior stop. Processes end on conversation change or Jecode exit; old IDs fail but saved output remains readable.",
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
                (
                    "output_limit_bytes",
                    integer(1, Some(super::processes::MAX_OUTPUT_LIMIT)),
                ),
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
        tool(
            "checkpoint",
            "List, inspect or restore before-state snapshots from modifying calls. write/edit save their file; Bash saves only declared protect paths. Refs expire 24 hours after capture. inspect returns an expected token for the selected current paths; omit paths for the saved call scope. restore needs that token for the same paths or expected set to latest to acknowledge live state. preview:true plans without writing; a real restore saves an undo. A receipt describes captured preimages, not command success.",
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
        routine(
            "web",
            "Use local curl for HTTP(S) fetch, raw download or link crawl. fetch returns status, final_url, MIME, raw/text paths and preview; extraction stops at 4 MiB (see extraction_truncated). HTML returns 100 links at most; links_count/links_path cover all. download saves raw bytes. fetch/download need url and accept method, headers, body; headers stay at the requested origin on redirects. crawl uses GET without body: start with urls or resume with crawl_id and unchanged saved seed headers, timeout_seconds and max_bytes. Defaults: same_origin true, max_depth 1, max_pages 20 per call. Returns page summaries plus manifest_path for full JSONL records; page with read. max_bytes limits each response only when set. Use browser for rendered pages or search sites.",
            Value::object([
                (
                    "action",
                    Value::object([
                        ("type", Value::string("string")),
                        (
                            "enum",
                            Value::Array(["fetch", "download", "crawl"].map(Value::string).into()),
                        ),
                    ]),
                ),
                ("url", string()),
                ("urls", strings()),
                ("crawl_id", string()),
                ("method", string()),
                (
                    "headers",
                    Value::object([
                        ("type", Value::string("object")),
                        ("additionalProperties", string()),
                    ]),
                ),
                ("body", string()),
                ("timeout_seconds", integer(1, None)),
                ("max_bytes", integer(1, None)),
                ("max_pages", integer(1, None)),
                ("max_depth", integer(0, None)),
                ("same_origin", boolean()),
            ]),
            &["action"],
        ),
        routine(
            "browser",
            "Control installed Chromium locally via CDP. open defaults to isolated headless mode; visible:true opens a window, and profile names persistent profiles (default temporary). Omitted browser_id selects newest; omitted tab_id selects an available tab, so specify it when needed. snapshot gives element refs such as @1; selector accepts refs or CSS. wait uses selector/text. screenshot saves tmp:browser/, delivers an attachment next request, and the ref survives temporary cleanup. cdp sends a raw DevTools method. Use visible mode for login or two-factor prompts. Chromium must be installed; JECODE_BROWSER can name its executable.",
            Value::object([
                (
                    "action",
                    Value::object([
                        ("type", Value::string("string")),
                        (
                            "enum",
                            Value::Array(
                                [
                                    "open",
                                    "tabs",
                                    "navigate",
                                    "snapshot",
                                    "click",
                                    "fill",
                                    "press",
                                    "scroll",
                                    "wait",
                                    "screenshot",
                                    "evaluate",
                                    "cdp",
                                    "logs",
                                    "close",
                                    "stop",
                                ]
                                .map(Value::string)
                                .into(),
                            ),
                        ),
                    ]),
                ),
                ("url", string()),
                ("visible", boolean()),
                ("profile", string()),
                ("browser_id", string()),
                ("tab_id", string()),
                ("selector", string()),
                ("text", string()),
                ("key", string()),
                ("expression", string()),
                ("method", string()),
                ("params", Value::object([("type", Value::string("object"))])),
                ("timeout_ms", integer(1, None)),
                ("x", integer(0, None)),
                ("y", integer(0, None)),
                (
                    "delta_y",
                    Value::object([("type", Value::string("integer"))]),
                ),
                ("width", integer(1, None)),
                ("height", integer(1, None)),
            ]),
            &["action"],
        ),
    ])
}

#[cfg(test)]
mod tests;
