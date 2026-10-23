use crate::json::Value;

/// Executable tools in Jecode's neutral name, description and parameters shape.
/// Each backend owns the wrapper required by its request protocol.
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
        tool(
            "web",
            "Use local curl to fetch an HTTP(S) URL, download its bytes, or crawl links from starting URLs. fetch returns metadata, a short text preview, complete raw path, and extracted text_path when available. Extraction reads at most 4 MiB; extraction_truncated marks a partial extraction. HTML returns the first 100 links, links_count and links_path to all links extracted from that portion. download returns raw metadata and path without parsing text. crawl returns crawl_id, pages_in_call, up to 20 page summaries, and manifest_path to full JSONL records; use read paging for the manifest. Continue with crawl_id. This is ordinary HTTP, not browser rendering or a search API. For public search, navigate a search site with browser. Fetch/download require url; crawl requires urls or crawl_id. max_bytes limits each raw response when supplied. Crawl defaults: same_origin true, max_depth 1, max_pages 20 per call. Site access depends on the target and local network.",
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
        tool(
            "browser",
            "Control an installed Chromium-based browser locally through the Chrome DevTools Protocol. open launches an isolated headless browser by default and optionally navigates to url; visible:true opens a window. profile names select dedicated persistent profiles, while the default profile is temporary. Actions: tabs, navigate, snapshot, click, fill, press, scroll, wait, screenshot, evaluate, cdp, logs, close, stop. An omitted browser_id selects the newest browser; an omitted tab_id selects an available page tab, so pass tab_id for a specific tab. snapshot returns element refs such as @1; selector accepts a ref or CSS. wait uses selector and/or text. screenshot saves a tmp:browser/ image, imports it as an attachment and delivers it on the next model request; its attachment reference survives temporary cleanup. cdp calls a raw DevTools method for advanced operations. Use a visible browser for login and two-factor prompts. Chromium must already be installed; JECODE_BROWSER may point to its executable.",
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
