# Tools

Jecode offers five tools to the model. A call runs only after the complete model
response and its arguments validate; streamed partial calls never execute.
Tool errors return as results so the model can correct the call.

## Tool arguments

| Tool | Required arguments | Optional arguments | Result or constraint |
| --- | --- | --- | --- |
| `read` | `path` | `offset`, `limit`, `byte_offset` | Pages UTF-8 text or shows an image/PDF; also reads saved output, history, skills and attachments. |
| `write` | `path`, `content` | — | Creates or replaces a complete UTF-8 file; returns `bytes_written`. |
| `edit` | `path`, `old_text`, `new_text` | — | Replaces exactly one nonempty match; returns `bytes_written`. A missing or ambiguous match leaves the file unchanged. |
| `bash` | `command` | `timeout_seconds`, `wait_ms` | Runs a Bash command from the project directory. With `wait_ms`, it returns a persistent `process_id`. |
| `process` | `action` | `process_id` for `poll`/`stop`, `wait_ms` for `poll` | Lists, polls or stops this conversation's persistent processes. |

## File paths and paged reads

Project file paths resolve within the working directory, including existing
symbolic links. Project paths passed to `read`, `write` and `edit` cannot cross
that boundary. A new project file needs an existing parent directory. `write`
and `edit` reject read-only files and replace accepted files through a sibling
temporary file. Writes, edits and text reads use UTF-8 without NUL bytes.

Images and PDFs in the project or the current `tmp:` area are recognized from
their contents and shown to the model on the next request, using the same
[delivery rules as attachments](ATTACHMENTS.md#what-the-model-receives).
The result returns an `attachment:ID` reference to a session-owned copy. Read
that reference to revisit it after the original changes, temporary cleanup or
session resume. Other binary or invalid UTF-8 file reads still fail.

The interactive TUI shows Read, Update (`edit`), Write, Bash and Process in one
inline format. Successful file changes retain a numbered contextual diff,
including Write on an existing text file. These display records stay in local
session history and survive resume without changing model-facing tool results.
Diff capture is optional: inputs above 1 MiB or 20,000 lines, and expensive
alignments, use a neutral content preview. These display limits never reject
a valid file modification.
Short diffs stay open when another tool starts. After the execution turn ends,
`Ctrl+O` expands every call from that turn, including long diffs and retained
output; another `Ctrl+O` restores the normal previews. The shortcut is ignored
during generation and does nothing when the latest finished turn has no tools.
See [TUI theme](../tui/THEME.md) for presentation and controls.

Text reads use one-based `offset` (default 1) and `limit` (default 200, maximum
2,000 lines). Pages are also capped at 64 KiB. Continue with `next_offset`; for
a line split by the byte cap, use `next_byte_offset` as `byte_offset`.
`byte_offset` is zero-based, takes precedence over `offset` and returns
unnumbered text with a byte range. `total_lines` appears at EOF in line mode.
These are page limits, not a file-size limit. Paging arguments do not crop
images or PDFs.

The `path` argument also accepts these read-only references:

| Reference | What it retrieves |
| --- | --- |
| `output:SESSION_ID:OUTPUT_ID:stdout` or `:stderr` | Complete saved Bash stream, including text omitted from a result preview. Older `output:OUTPUT_ID:stdout` and `:stderr` references also work. |
| `history:N` | Original message at zero-based index `N`. A `context_truncated` result includes its `history_reference`. |
| `history:requests` / `history:memory` | Ordered original user requests / current compaction summary. |
| `skill:NAME` / `skill:NAME/relative/path` | A personal skill's `SKILL.md` / a resource inside that skill. See [Skills](SKILLS.md). |
| `attachment:ID` | An attached text file, or a re-shown image/PDF; other binaries return their local path. See [Attachments](ATTACHMENTS.md#what-the-model-receives). |

History pages remain available after compaction and resume and exclude
provider-specific reasoning fields. Reading a tool result can also return
`request_history_reference` for the original tool call, including exact `write`
content. Indexed pages identify the source role and originating request/call.
Known credentials are masked. History, skills, attachments and saved output cannot be
changed with `write` or `edit`.

For disposable files, use `tmp:relative/path` with the file tools. Its parent
directories are created as needed, and Bash receives `JECODE_TMP`, `TMPDIR`,
`TEMP` and `TMP` pointing to the session's temporary area. Bash still starts in
the project directory, so use an explicit path such as `"$JECODE_TMP/check.log"`
for disposable output. See [Temporary files](TEMPORARY.md) for lifetime and
cleanup.

## Bash execution and saved output

`bash` starts a fresh `bash -c` process with closed stdin and no shared shell
state. It has normal system access; the project working directory is not a
sandbox. Startup profiles are disabled, and Jecode removes `BASH_ENV`, `ENV`
and `OPENROUTER_API_KEY` from the command environment. No timeout is imposed
unless `timeout_seconds` is a positive integer, measured from command start.

Without `wait_ms`, the call waits for completion. It returns `exit_code`,
`cancelled`, `timed_out`, byte counts, and the last 64 KiB of each stdout/stderr
stream. `stdout_file` and `stderr_file` point to the complete saved streams;
read those references when a preview is truncated. Known configured keys are
masked before storage. If saving output or confirming cleanup fails after a
command starts, the result reports `outcome: "unknown"` with output references.
Inspect the workspace before repeating a command that may have changed it.

## Persistent processes

Pass `wait_ms` to `bash` for a server, watcher or long build. Jecode waits up to
that many milliseconds and returns a `process_id` plus status; `wait_ms: 0`
returns immediately. Expiring the wait does not stop the process. There is no
PTY or later stdin input. An optional `timeout_seconds` remains a separate
deadline from launch.

Tool results describe the state when the call returned. The TUI labels a Bash
launch `Started in background` and a running Process check `Running at last
check`; these cards are snapshots. Use `/terminal` for the current active list.

`process` actions are:

| Action | Behavior |
| --- | --- |
| `list` | Returns IDs, commands, statuses and output references for this conversation. Use it to recover an ID after compaction. |
| `poll` | With `process_id`, returns current status and new output since the previous result; optional `wait_ms` defaults to 0. |
| `stop` | With `process_id`, requests termination of the process and descendants and waits up to 10 seconds. If the result is `stopping`, poll again to confirm. |

Statuses are `running`, `stopping`, `exited`, `timed_out`, `stopped` and
`unknown`. Only a finished `exited` process has an `exit_code` (possibly
`null`). `unknown` means completion or cleanup could not be confirmed; inspect
the workspace before retrying. An empty output preview says nothing about
readiness or completion: check `status`.

Each result includes at most the last 64 KiB of *new* output per stream.
`stdout_offset`/`stderr_offset` identify the start of returned text;
`stdout_next_offset`/`stderr_next_offset` identify the next cursor; and
`stdout_skipped_bytes`/`stderr_skipped_bytes` count earlier unread bytes omitted
from the preview. Read the corresponding `*_file` with `byte_offset` to retrieve
those bytes. Interrupting a `poll` wait reports `wait_cancelled: true` and
leaves the process running; interrupting a `stop` wait does not undo the stop.

Processes survive tool calls and turns, but end on `process stop`, Jecode exit,
a successful conversation switch or deletion of the live session. Resume does
not restart them; their saved output remains readable. Jecode does not notify
the model when a process changes, so poll when its status matters. Native
supervision stops owned descendants on timeout or cancellation; see
[Architecture](../development/ARCHITECTURE.md) for the platform boundary.

### Viewing and stopping processes

`/terminal` lets you inspect or stop processes locally without adding a model
message or consuming the model's `process poll` output cursor. In the TUI, it
opens the process list; Enter shows output, Tab switches stdout/stderr, and
Ctrl+D asks for confirmation before stopping the selected process.
`/terminal ID` opens a known process directly, and `/terminal stop ID` requests
its stop. In plain chat, `/terminal` lists active processes and `/terminal ID`
shows status and the last 64 KiB of each stream. A stop request can remain
pending; check again for its final status.

For connection retries and context compaction during long turns, see
[Long-running work](LONG_WORK.md).
