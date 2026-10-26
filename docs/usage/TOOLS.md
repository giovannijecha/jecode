# Tools

Jecode offers eight tools to the model. An assistant message describing a command
or file change does not execute it; only a native tool call can do that. Check
the tool result or resulting state before treating a change as complete. Calls
run only after the complete model response arrives; streamed partial calls never
execute. Up to four adjacent independent calls run together: `read` except
`history:`/`skill:`, `checkpoint list`/`inspect`, `process list`, and
`web fetch`/`download` with GET or HEAD and no body. Other calls end the batch,
including process polling, crawls and workspace modifications. Results enter the
conversation and saved session in call order before the next batch or tool runs.
Tool errors return as results so the model can correct the call.

## Tool arguments

| Tool | Required arguments | Optional arguments | Result or constraint |
| --- | --- | --- | --- |
| `read` | `path` | `offset`, `limit`, `byte_offset`, `work`, `description` | Pages UTF-8 text or shows an image/PDF; also reads saved output, history, skills and attachments. |
| `write` | `path`, `content` | — | Creates or replaces a complete UTF-8 file; returns `bytes_written`. |
| `edit` | `path`, `edits` | — | Applies one or more exact unique replacements in order, installing the file only after all succeed. Returns `bytes_written`. |
| `bash` | `command` | `protect`, `timeout_seconds`, `wait_ms`, `output_limit_bytes`, `work`, `description` | Runs a Bash command from the project directory. `protect` saves declared files or recursive directory scopes before launch. With `wait_ms`, it returns a persistent `process_id`. |
| `process` | `action` | `process_id` for `poll`/`stop`, `wait_ms` for `poll`, `output_limit_bytes`, `output_mode`, `work`, `description` | Lists, polls or stops this conversation's persistent processes. |
| `checkpoint` | `action` | `ref`, `paths`, `expected`, `preview`, `limit` | Lists, inspects and restores the before state of individual modifying calls. See [Checkpoints](CHECKPOINTS.md#inspect-and-recover). |
| `web` | `action`; `url` for `fetch`/`download`, `urls` or `crawl_id` for `crawl` | `method`, `headers`, `body`, `timeout_seconds`, `max_bytes`, `max_pages`, `max_depth`, `same_origin`, `work`, `description` | Fetches HTTP(S) resources and crawls pages with local curl; saves bodies under `tmp:web/`. |
| `browser` | `action`; `url` for `navigate`, `expression` for `evaluate`, `method` for `cdp` | `url`, `visible`, `profile`, `browser_id`, `tab_id`, `selector`, `text`, `key`, `params`, `timeout_ms`, `x`, `y`, `delta_y`, `width`, `height`, `work`, `description` | Controls an installed Chromium-based browser through local DevTools; screenshots and logs go under `tmp:browser/`. |

`work` is an optional short label chosen by the assistant for related `read`,
`bash`, `process`, `web` and `browser` calls. At least two adjacent eligible calls
share a TUI Work chapter only within the same execution turn and with matching
normalized labels. A single eligible call keeps its normal compact tool card.
A changed label, assistant narration or another tool breaks the chapter; skill
reads stay separate. Missing or blank labels appear as Activity, including in
older sessions. Displayed labels collapse whitespace to one line and are
ellipsized to at most 60 terminal cells; the saved label stays unchanged and
grouping compares the same normalized label before display truncation. Labels
are saved with their original tool arguments and survive resume; they do not
change execution. See
[tool presentation](../tui/THEME.md) for the summary and `Ctrl+O` behavior.

`description` is an optional short caption on those same five routine tools.
It is stored unchanged with the original tool arguments alongside `work` and
survives resume; it does not change execution. Bash and Process use it as the
compact title above their command. When omitted, the command or tool target
remains the visible fallback. Browser keeps its action in the title and shows
its payload beneath it. See [tool presentation](../tui/THEME.md) for the layout.

`write`, `edit` and protected `bash` calls return a `checkpoint` receipt whose
`status` is `saved`, `partial` or `none`. Saved references expire 24 hours after
capture; see [Checkpoints](CHECKPOINTS.md#what-is-saved).

## File paths and paged reads

Project file paths resolve within the working directory, including existing
symbolic links. Project paths passed to `read`, `write` and `edit` cannot cross
that boundary. A new project file needs an existing parent directory. `write`
and `edit` reject read-only files and replace accepted files through a sibling
temporary file. Writes, edits and text reads use UTF-8 without NUL bytes.

Use `edits: [{"old_text": "before", "new_text": "after"}, ...]` for one or more
replacements in one file. The executor also accepts a top-level
`old_text`/`new_text` pair for models that imitate older conversation history;
interrupted saved calls are not replayed. Empty or null unused legacy fields do
not conflict with a valid batch, but populated competing formats are rejected.
Each step matches the result of preceding steps, including exact
whitespace and line endings. A missing or ambiguous match leaves the original
file intact. A successful batch installs once and captures one workspace
checkpoint. Large-file batches keep original ranges and replacement text in a
virtual view, so later edits can match earlier replacements without writing
intermediate files. Each edit scans that view to verify a unique match; after
all matches succeed, the final content is written once to a temporary file,
synced and installed atomically. Buffering stays independent of file size;
range metadata grows with the number of edits.

Images and PDFs in the project or the current `tmp:` area are recognized from
their contents and shown to the model on the next request, using the same
[delivery rules as attachments](ATTACHMENTS.md#what-the-model-receives).
The result returns an `attachment:ID` reference to a session-owned copy. Read
that reference to revisit it after the original changes, temporary cleanup or
session resume. Other binary or invalid UTF-8 file reads still fail.

The interactive TUI shows Read, Update (`edit`), Write, Bash, Process, Checkpoint,
Web and Browser in one
inline format. Successful file changes retain a numbered contextual diff,
including Write on an existing text file. These display records stay in local
session history and survive resume without changing model-facing tool results.
The four-cell indent stays on the terminal surface; changed rows carry a red
or green tint through the right edge. Paired changed fragments use a stronger
surface, while unpaired code keeps its syntax colours without whole-line bold.
Context remains dim and unshaded. A new Write shows `New file · N lines` and
the first six numbered source lines instead of an all-added diff; Ctrl+O
reveals the complete new file.
Diff capture is optional: inputs above 1 MiB or 20,000 lines, and expensive
alignments, use a neutral content preview. These display limits never reject
a valid file modification.
Short diffs stay open when another tool starts. After the execution turn ends,
`Ctrl+O` expands every call from that turn, including long diffs and retained
output; another `Ctrl+O` restores compact single-call previews and chapter
summaries. Every Bash target in a multi-call chapter stays visible even after
success, including in older turns. The shortcut is ignored
during generation and does nothing when the latest finished turn has no tools.
It keeps your reading position and briefly confirms the change with the
number of completed tools.
Compact Work chapters keep their facts row and one shared activity row for
the latest Read, Web or Browser attempt, including failures; the activity
stays visible after completion. The chapter's `Ctrl+O expand` keycap shares
its title row. Bash and Process members keep one quiet compact row; Ctrl+O
opens their process IDs and retained output. A compact Read summarizes its
page on one line, naming the total when known or the next page otherwise.
See [TUI theme](../tui/THEME.md) for presentation and controls.

## Local web and browser tools

`web` uses the system's HTTPS-enabled `curl`, directly from the user's machine.
`fetch` returns the HTTP status, final URL, MIME type, response size, a text
preview when available, and a `tmp:web/` path to the complete raw response.
For text and HTML, extraction reads at most the first 4 MiB of that response;
`extraction_truncated` marks an incomplete extraction. `text_path` holds the
extracted text. For HTML, `links` contains at most 100 links; `links_count` and
`links_path` give the count and JSON file of every link extracted from the
portion read. The raw response is stored in full; a response over `max_bytes`
fails the call.
`download` stores the raw response and returns its `tmp:web/` path and metadata
without text or link extraction. Requests use GET by default, or POST when
`body` is supplied without `method`. Set `method`, string-valued `headers`,
`body`, `timeout_seconds` and `max_bytes` as needed. A missing `max_bytes` does
not impose a size limit. The saved files can be read with `read` or used with
`bash`; they follow the active [session temporary area](TEMPORARY.md).
HEAD returns metadata and saves the response headers without reading a body.
POST redirects use GET after 301, 302 or 303; 307 and 308 preserve the method
and body. Other methods use GET after 303, except HEAD. Custom headers follow
redirects only within the requested origin (scheme, host and port).

`crawl` starts with `urls` or continues with a returned `crawl_id`. It extracts
links from HTML and reports `pages_in_call`, `pages_total`, `queued` and
`complete`. `pages` contains at most 20 compact summaries; when
`pages_truncated` is true, read `manifest_path` for the full records. That
append-only JSONL file contains one `{page,discovered}` object per fetched page;
page it with `read`. On resume, `manifest_repaired` reports recovery of an
incomplete last record. `same_origin` defaults to true, `max_depth` to 1 and
`max_pages` to 20 per call. Crawls use GET without a body; use `fetch` or
`download` for requests with other methods. Headers are sent only to the seed
origins, including during redirects, even when `same_origin` is false.
Headers, `timeout_seconds` and `max_bytes` are saved with the crawl and apply
to each page. Resume with `crawl_id` and optionally `max_pages`; conflicting
request options require a new crawl. An omitted `max_bytes` leaves downloads
unlimited. Crawl state, including supplied headers, and fetched artifacts stay
in the active temporary area.
Authentication, dynamic content and network errors can limit the pages returned.
For a public search, use `browser` to navigate to a search site; Jecode
provides no search service or search API.

`browser` uses an already installed Chromium-based browser. Its `open` action
starts headless by default; `visible: true` opens a window.
Without `profile`, it uses an isolated temporary profile. A named `profile`
uses a dedicated persistent profile under `~/.jecode/browser/profiles/`; it can
retain cookies and signed-in sessions. Use a visible session for login and
two-factor prompts. Set `JECODE_BROWSER` to a local browser executable when
auto-discovery cannot find one. Jecode does not install a browser.

The browser actions are `open`, `tabs`, `navigate`, `snapshot`, `click`, `fill`,
`press`, `scroll`, `wait`, `screenshot`, `evaluate`, `cdp`, `logs`, `close` and
`stop`. `open` returns `browser_id` and `tab_id`. The newest browser is selected
when `browser_id` is omitted. Without `tab_id`, Jecode selects an available page
tab; pass an explicit ID when the tab matters. `snapshot` returns the page text, interactive
elements and references such as `@1`; `selector` accepts a snapshot reference
or CSS selector in interaction actions. `wait` matches `text` in the selected
element or the page body, and accepts `timeout_ms`. `scroll` takes signed
`delta_y` and optional `x`/`y` coordinates. `evaluate` runs a JavaScript
`expression` on the page. `cdp` accepts a raw DevTools `method` and object
`params` for operations beyond the high-level actions. `screenshot` saves a
`tmp:browser/` image and automatically delivers it to the model on the next
request. The returned `attachment:ID` reference remains usable after temporary
cleanup. `logs` returns
up to 30 recent events and a saved path containing the current connection's
retained event window (up to 1,000 events). `close` closes a tab; `stop` ends a browser.
Browser behavior depends on the target site and its current state.

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
| `history:N` | Original message at zero-based index `N`, excluding input withdrawn to the composer. A `context_truncated` result includes its `history_reference`. |
| `history:requests` / `history:memory` | Ordered original user requests / current compaction summary. |
| `skill:NAME` / `skill:NAME/relative/path` | A personal skill's `SKILL.md` / a resource inside that skill. See [Skills](SKILLS.md). |
| `attachment:ID` | An attached text file, or a re-shown image/PDF; other binaries return their local path. See [Attachments](ATTACHMENTS.md#what-the-model-receives). |
| `checkpoint:rc-ID:path` | A file saved before one modifying call. See [Checkpoints](CHECKPOINTS.md). |

History pages remain available after compaction and resume and exclude
provider-specific reasoning fields. Reading a tool result can also return
`request_history_reference` for the original tool call, including exact `write`
content. Indexed pages identify the source role and originating request/call.
Known credentials are masked. History, checkpoints, skills, attachments and saved
output cannot be changed with `write` or `edit`.

For disposable files, use `tmp:relative/path` with the file tools. Its parent
directories are created as needed, and Bash receives `JECODE_TMP`, `TMPDIR`,
`TEMP` and `TMP` pointing to the session's temporary area. Bash still starts in
the project directory, so use an explicit path such as `"$JECODE_TMP/check.log"`
for disposable output. See [Temporary files](TEMPORARY.md) for lifetime and
cleanup.

## Bash execution and saved output

`bash` starts a fresh `bash -c` process with closed stdin and no shared shell
state. It has normal system access; the project working directory is not a
sandbox. Startup profiles are disabled, and Jecode removes `BASH_ENV`, `ENV`,
`OPENROUTER_API_KEY` and `JECODE_OPENAI_ACCESS_TOKEN` from the command
environment. No timeout is imposed unless `timeout_seconds` is a positive
integer, measured from command start.
Commands use Bash syntax on Windows too; use an explicit PowerShell invocation
when a PowerShell command is needed.

Without `wait_ms`, the call waits for completion. It returns `exit_code`,
`cancelled`, `timed_out`, byte counts, and a 16 KiB preview of each stdout/stderr
stream. Truncated previews retain the beginning and end, with an omitted-byte
marker. Set a positive `output_limit_bytes` to request a smaller or larger
per-stream preview, up to 256 KiB. Larger values are clamped before reading
the logs; the result's `output_limit_bytes` reports the effective budget.
This bounds the preview and its allocation, not command execution or saved logs.
`stdout_file` and `stderr_file` point to the complete saved streams; read those
references when a preview is truncated. Known credentials, including the
OpenRouter key and registered ChatGPT tokens, are masked before storage. If
saving output or confirming cleanup fails after a command starts, the result
reports `outcome: "unknown"` with output references.
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

Each result reads the next 16 KiB of saved output per stream by default;
`output_limit_bytes` changes this budget, capped at 256 KiB, for the initial
Bash result or a `process poll`/`stop`. A budget smaller than one UTF-8
character still returns that complete character so the cursor can advance.
`stdout_offset`/`stderr_offset` identify the start of returned text;
`stdout_next_offset`/`stderr_next_offset` identify the next cursor; and
`stdout_remaining_bytes`/`stderr_remaining_bytes` count unread saved data at
the time of the result, including when the process has already exited.
With the default `output_mode: "next"`, `stdout_skipped_bytes` and
`stderr_skipped_bytes` remain zero: polling never skips unread output. Poll
again for the next window or read the corresponding `*_file` with `byte_offset`.

For readiness/status checks behind a large backlog, pass `output_mode: "tail"`
to `process poll` or `stop`. It returns the newest unread bytes and advances
the cursor past that window. `*_skipped_bytes` counts the earlier unread data
omitted from this result; the complete saved log remains readable. The mode
never changes automatically with backlog size. A later `next` poll continues
from the returned `*_next_offset`, without replaying the tail.

Interrupting a `poll` wait reports `wait_cancelled: true` and
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
