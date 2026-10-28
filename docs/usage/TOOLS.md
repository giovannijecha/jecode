# Tools

Jecode offers thirteen tools to the main model and twelve operational tools to
collaborators, which cannot delegate. An assistant message describing a command
or file change does not execute it; only a native tool call can do that. Check
the tool result or resulting state before treating a change as complete. Calls
run only after the complete model response arrives; streamed partial calls never
execute. Adjacent eligible reads run together automatically, up to four at a
time: `list`, `search`, `read` except `history:`/`skill:`, `map overview`,
`checkpoint list`/`inspect`, `process list`, and `web fetch`/`download` with GET
or HEAD and no body. The model can also declare adjacent calls independent with
`parallel: true`, including writes and commands. Results enter the conversation
and saved session in call order. Tool errors return as individual results so the
model can correct the call.

## Tool arguments

All tools accept an optional boolean `parallel` argument. The `agent` control
tool always runs in order; its workers run independently after creation. The
table lists the other arguments.

| Tool | Required arguments | Optional arguments | Result or constraint |
| --- | --- | --- | --- |
| `read` | `path` | `offset`, `limit`, `byte_offset`, `work`, `description` | Pages UTF-8 text or shows an image/PDF; also reads saved output, history, skills and attachments. |
| `write` | `path`, `content` | — | Creates or replaces a complete UTF-8 file; returns `bytes_written`. |
| `edit` | `path`, `edits` | — | Applies one or more exact unique replacements in order, installing the file only after all succeed. Returns `bytes_written`. |
| `bash` | `command` | `protect`, `timeout_seconds`, `wait_ms`, `output_limit_bytes`, `work`, `description` | Runs a Bash command from the project directory. `protect` saves declared files or recursive directory scopes before launch. With `wait_ms`, it returns a persistent `process_id`. |
| `process` | `action` | `process_id` for actions except `list`; `text` or `key` for `write`; `cols`, `rows` for `resize`; `protect` for `write`/`resize`/`interrupt`; `wait_ms` for `poll`; `output_limit_bytes`, `output_mode`, `work`, `description` | Lists, polls, stops and controls this conversation's persistent processes. |
| `checkpoint` | `action` | `ref`, `paths`, `expected`, `preview`, `limit` | Lists, inspects and restores the before state of individual modifying calls. See [Checkpoints](CHECKPOINTS.md#inspect-and-recover). |
| `web` | `action`; `url` for `fetch`/`download`, `urls` or `crawl_id` for `crawl` | `method`, `headers`, `body`, `timeout_seconds`, `max_bytes`, `max_pages`, `max_depth`, `same_origin`, `work`, `description` | Fetches HTTP(S) resources and crawls pages with local curl; saves bodies under `tmp:web/`. |
| `browser` | `action`; `url` for `navigate`, `expression` for `evaluate`, `method` for `cdp` | `url`, `visible`, `profile`, `browser_id`, `tab_id`, `selector`, `text`, `key`, `params`, `timeout_ms`, `x`, `y`, `delta_y`, `width`, `height`, `filter`, `limit`, `cursor`, `work`, `description` | Controls an installed Chromium-based browser through local DevTools; screenshots and logs go under `tmp:browser/`. |
| `exec` | `executable` | `args`, `cwd`, `env`, `stdin`, `tty`, `cols`, `rows`, `protect`, `wait_ms`, `timeout_seconds`, `output_limit_bytes`, `work`, `description` | Runs an executable with separate arguments and no shell parsing. `tty:true` returns a controllable terminal process. |
| `list` | None | `path`, `recursive`, `pattern`, `include_hidden`, `include_ignored`, `offset`, `limit`, `work`, `description` | Lists project entries, optionally as a recursive tree, with types and workspace-relative paths. |
| `search` | `query` | `path`, `pattern`, `case_sensitive`, `context`, `include_hidden`, `include_ignored`, `offset`, `limit`, `work`, `description` | Finds literal text in UTF-8 project files; returns line numbers, context and byte offsets. |
| `map` | None | `action`, `path`, `query`, `pattern`, `offset`, `limit`, `include_hidden`, `include_ignored`, `id`, `title`, `summary`, `sources`, `work`, `description` | Surveys the project on demand and manages source-backed project notes. `action` defaults to `overview`; `remember` requires `title`, `summary` and `sources` at execution. |

| `agent` | `action`; action-specific fields below | `tasks`, `id`, `instructions`, `context`, `timeout_ms`, `report_version`, `note`, `paths`, `expected_review_token`, `path`, `offset`, `limit` | Main agent only: create, update, inspect, wait, stop, review, compare, integrate or close collaborators. See [collaboration](COLLABORATION.md). |
| `ask_user` | `questions` (1–4: `id`, `title`, `question`) | per question: `context`, `multiple`, `options` | Ask up to four questions, answered one at a time, with single or multiple choice and the user's own words; wait for explicit answers. Main agent only. See [questions and plans](WORK_TOOLS.md). |
| `todo_list` | `action`; `title` and `tasks` for `set` | None | Get or atomically replace this conversation's plan. Successful updates create immutable numbered revisions. See [questions and plans](WORK_TOOLS.md). |

## Parallel calls

Set `parallel: true` on each adjacent call whose inputs and effects are
independent. Jecode runs at most four at once; longer runs proceed in bounded
waves. A call without `parallel: true` ends that declared group and waits for it
to finish. `parallel: false` always runs in order. With the argument omitted,
eligible reads keep their automatic batching and modifying calls run in order.
Declared groups and automatic read groups never merge. `history:` and `skill:`
reads and `agent` control calls remain ordered even with `parallel: true`.

The model must consider shared files, interfaces, generated output, processes
and external state before declaring independence. Different paths alone do not
establish it; Jecode does not infer it from paths. Each call has its own result.
Workspace checkpoints belong to individual modifying calls; commands save only
declared `protect` paths. One failure does not undo another call or make a
multi-file operation atomic. A `bash` or
`exec` result with `process_id` only confirms launch or the end of its requested
wait; poll the process before work
that depends on its completion.

For concurrent commands, use focused `protect` paths for each command's
recovery scope. Restoring a broad scope can also undo another operation's
changes. In explicitly declared groups, completed calls appear individually as
they finish. Their
results are saved before display; the next model request receives all results
in original call order. If Jecode closes mid-group, saved completions retain
their results and diffs, started calls without a saved result have an unknown
outcome, and calls not started are marked not executed. None are automatically
replayed; inspect current state before deciding what to retry.

`work` is an optional short label chosen by the assistant for related `read`,
`list`, `search`, `map`, `bash`, `process`, `web`, `browser` and `exec` calls. At least two adjacent eligible calls
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

`description` is an optional short caption on those same nine routine tools.
It is stored unchanged with the original tool arguments alongside `work` and
survives resume; it does not change execution. Bash and Process use it as the
compact title above their command. When omitted, the command or tool target
remains the visible fallback. Browser keeps its action in the title and shows
its payload beneath it. See [tool presentation](../tui/THEME.md) for the layout.

## Native file discovery

`list` and `search` use Jecode's own Rust implementation; neither requires an
installed search program or shell. Paths default to the project root (`.`),
and returned paths are always workspace-relative with `/` separators. Absolute
paths inside the project are accepted; paths outside it are rejected.

Examples of tool arguments:

```json
{"path":"src","recursive":true,"pattern":"*.rs"}
```

The `list` call above returns Rust file entries under `src`. Without a pattern,
it also returns directories and links. `recursive` defaults to `false`;
immediate children have depth 1. Links and Windows junctions are listed without
descending into them.

```json
{"query":"Cancellation","path":"src","pattern":"src/**/*.rs","context":2}
```

The `search` call above returns one result per matching line, including the
first matched byte's `byte_offset`, a 1-based `line`, `text` and `before`/`after`
context. Search is recursive under directories; `path` can also select one file.
`query` is literal nonempty single-line UTF-8 text. `case_sensitive` defaults to
`true`; `false` ignores ASCII case only. `context` defaults to 2, from 0 to 10.
Use Bash or exec with an installed program for regular-expression searches.

Both tools accept a path glob in `pattern`: `*`, `?`, `**` and bracket classes.
A pattern without `/` matches an entry's basename; one containing `/` matches
its workspace-relative path, even when `path` selects a subdirectory. Windows
matching folds ASCII case. Patterns filter results without pruning unmatched
directories from recursive exploration.

Hidden dot-prefixed entries and ignored files/directories are excluded by
default. `include_hidden` and `include_ignored` opt in independently. Ignore rules
come from local and repository-ancestor `.gitignore` files and repository
`info/exclude`, including Git worktrees; the global Git excludes file is not
read. Git metadata is omitted unless `include_ignored` is true. Selecting an
explicit path bypasses that path's own exclusions, while descendants still use
normal filtering. Implicit traversal never follows links or junctions.

`offset` is 1-based and defaults to 1. `list` defaults to 200 entries per page
(maximum 2000); `search` defaults to 100 matching lines (maximum 1000). Encoded
results are capped at 64 KiB, so a page can contain fewer results than `limit`.
Repeat the same arguments with `next_offset` until it is null. Traversal uses
deterministic depth-first path order and ascending line order, reading live
files on each call; restart pagination after changes to the workspace.

Long lines are searched in streaming chunks. Their bounded previews center on
the first match, with `text_truncated` marking omitted text. Use `read` with the
returned path and `byte_offset` to recover source text without a temporary script.
Binary and non-UTF-8 files are counted in `skipped`; invalid tails after valid
lines produce an error while retaining earlier matches. Unreadable paths also
appear in `errors`. Error details are bounded; `errors_count` and
`errors_truncated` describe any omitted details. `files_searched` counts opened
candidate files, including candidates subsequently identified as non-text.

`complete` is false when more results are available or errors prevented a full
search. `skipped` reports exclusions separately. An empty result proves absence
only within the selected and readable text scope. Compact TUI cards show counts,
continuation and errors; `Ctrl+O` expands structured entries, matches and context.

## Project map

Call `map` with no arguments for an overview of the current project. The map is
built when called; Jecode does not scan the project or put a complete map in the
starting model context. The overview surveys candidate paths and extracts a
focused lexical outline from Rust and Markdown files. It identifies areas,
landmarks and symbols without claiming a language-wide call graph.

The root overview stays coarse. Selecting a directory also reveals its
immediate Rust source files as module candidates, so follow an area with
`path` before focusing an individual file for its outline.

```json
{"action":"overview","path":"src","query":"config","limit":50}
```

`path` defaults to the project root. `pattern`, `include_hidden` and
`include_ignored` use the same path filtering as `list` and `search`. `query`
matches literal text in paths, titles, summaries and roles without case
sensitivity. Pagination uses a 1-based `offset` (default 1) and `limit` (default
100, maximum 1000). The encoded result is capped at 64 KiB; continue with a
numeric `next_offset` and identical arguments. Inventory collection processes
at most 100,000 candidate entries, keeps 2,000 rows, reads up to 80 candidate
headers of 8 KiB each, and reads at most 4 MiB for a focused outline.
`inventory_truncated` means narrow the path or query to recover omitted areas;
pagination alone cannot recover an inventory cut. Results include `entries`,
`entries_returned`, `files`, `notes_count`, `complete`, `errors`, `errors_count`
and `skipped`. Each entry identifies its kind (`area`, `landmark`, `symbol` or
`note`) and origin (`filesystem`, `source` or `model`).

Filename roles are candidates and lexical declarations are source anchors;
saved explanations remain model interpretations. Notes are read on demand.
Unreadable note storage preserves the structural result with
`notes_available:false`, `notes_count:null`, errors and `complete:false`.

The assistant can save a project note with `remember`:

```json
{"action":"remember","title":"Configuration entry point","summary":"Setup reads the user-scoped config here.","sources":[{"path":"src/config.rs","line":1}]}
```

`title`, `summary` and at least one source path are required. A source can omit
`line` to refer to the whole file; when supplied, the line is 1-based. The
receipt returns an `id`; pass that `id` to `remember` to update the note or to
`{"action":"forget","id":"..."}` to remove it. Notes are saved per project in
the Jecode home, separately from conversations. After a source changes, the
next overview reports its freshness as `changed`; absent sources report
`missing`. `current` means the full source content still matches the saved
fingerprint, while `unverified` means Jecode could not establish that state.
Inspect the cited files again before relying on a changed or unverified note.

Freshness checks only the notes on the requested page and references at most
64 MiB of source content per call; sources beyond that budget are `unverified`.
`remember` needs complete fingerprints within the same budget. Unchanged
references do not establish semantic correctness or cover unreferenced
dependencies. A note holds at most 16 sources, a 200-byte title and an
8 KiB summary; the store holds at most 256 notes and 8 MiB. Writes coordinate
across processes and preserve unrelated notes. Source paths and excerpts stay
available for focused reads even when the saved explanation needs revisiting.

## File recovery receipts

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

The interactive TUI shows Read, Update (`edit`), Write, Bash, Exec, Process,
Checkpoint, Web and Browser in one
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
its title row. Bash, Exec and Process members keep one quiet compact row; Ctrl+O
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
tab; pass an explicit ID when the tab matters.

`width` and `height` set the page viewport in CSS pixels (`innerWidth` and
`innerHeight`), not the outer window size. Supply both or neither; each is an
integer from 1 to 10,000. On `open`, the viewport applies to the first tab
before the optional `url` loads. On `screenshot`, it applies before capture.
The size stays with the tab across navigation and DevTools reconnection until
another size is requested, and the result's `viewport` reports the measured
size. Without them, `open` keeps the browser's default viewport and returns no
`viewport`; a screenshot captures the current viewport.

`snapshot` returns the page text separately from interactive elements. Each
element has a reference such as `@1`, its tag and visibility, Chromium's
computed `role` and accessible `name` (covering `<label for>`, wrapping labels,
`aria-label` and `aria-labelledby`), and, when present, `type`, `id`,
`html_name` (the `name` attribute), `placeholder`, its own `text` when it
differs from the name, the current `value` of text fields and selects (never
passwords), `checked` and `disabled`. Accessibility details cover the first 500
elements. Element strings are capped at 200 Unicode characters; field values
retain their whitespace. The response lists at most 100 elements, with
`elements_total` when more exist, and 12,000 characters of page text;
`snapshot_path` saves the full
snapshot. `selector` accepts a snapshot reference or CSS selector in
interaction actions.

`wait` matches `text` in the selected element or the page body, and accepts
`timeout_ms` (default 15,000). Every check uses only the remaining budget. An
unmet condition fails with the configured timeout, the elapsed time, the tab,
the last known URL and whether the selector matched nothing or the text was
missing. Cancellation, JavaScript errors and DevTools failures keep their own
errors. `scroll` takes signed
`delta_y` and optional `x`/`y` coordinates. `evaluate` runs a JavaScript
`expression` on the page. `cdp` accepts a raw DevTools `method` and object
`params` for operations beyond the high-level actions. `screenshot` saves a
`tmp:browser/` image and automatically delivers it to the model on the next
request. The returned `attachment:ID` reference remains usable after temporary
cleanup.

`logs` reads the selected tab's event log in pages. Capture starts when Jecode
first connects to the tab and keeps the newest 1,000 events, so it is not a
complete browser-session log. `filter` selects `default` (console messages,
exceptions, failed requests and HTTP errors), `console`, `errors`, `network`
or `all`. `limit` caps entries per page (default 50, maximum 200). Each entry
has `seq`, `kind`, `level`, `text` and, when known, `source`. Pass the returned
`next_cursor` as `cursor` to read only later events; `more` means retained
events after that cursor are still unread. `missed` counts events evicted
before this read could examine them. If the DevTools connection is recreated,
the log continues with a `gap` entry for events that could not be captured. A
malformed cursor, a future cursor or one from another tab or browser session is
an error; omit `cursor` to start from the oldest retained event.
Each page also examines at most 200 raw events or about 256 KiB of raw event
JSON; a single larger event is included so the cursor can advance. A page can
contain fewer than `limit` entries, even zero, while `more` is true. Continue
with `next_cursor` to examine the remaining retained events.
`logs_path` saves one JSON line for every event the page examined, including
non-matching ones, with the raw CDP message in `event`. `close` closes a tab;
`stop` ends a browser.
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
| `output:SESSION_ID:OUTPUT_ID:stdout` or `:stderr` | Complete saved command stream, including text omitted from a result preview. Older `output:OUTPUT_ID:stdout` and `:stderr` references also work. |
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
directories are created as needed, and command tools receive `JECODE_TMP` and
`TMPDIR` pointing to the session's temporary area. On Unix, `TEMP` and `TMP`
also point there; on Windows, native `TEMP` and `TMP` stay inherited so Git Bash
can initialize its shared `/tmp` mount. Bash still starts in
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

## Direct execution and terminals

`exec` runs `executable` directly. `args` is an array of literal argument strings:
spaces, quotes and shell metacharacters are passed unchanged. Jecode does not
start an implicit shell or rewrite the executable. A bare name follows normal
PATH search, which can select a shim already on PATH. On Windows, only the
effective PATH entries are searched in order: the project directory and system
directories are not implicitly searched before them. A bare name without a dot
is searched as `name.exe`. Dotted names such as `python3.11` try the exact name
before adding `.exe`; explicit paths remain literal.
Executable and argument strings cannot contain NUL characters.
The process starts in the project directory unless `cwd` names an absolute
directory or a directory relative to the project. An executable path containing
a directory is resolved against that `cwd`; a bare name uses normal PATH search.
Changing `cwd` does not change the project root or checkpoint scope.

`env` maps names to strings to set or `null` to remove. Jecode removes inherited
provider credentials first, then applies explicit `env` values. `stdin` sends a
UTF-8 string and closes the stream; omitted `stdin` closes it immediately.
Session temporary paths are available as `JECODE_TMP` and `TMPDIR`, with `TEMP`
and `TMP` also set on Unix. On Windows those two native variables remain
inherited to preserve Git Bash's `/tmp` mount.

For a command that needs interactive input, set `tty:true`. This always returns
a persistent `process_id`, even when `wait_ms` is omitted. `stdin` cannot be
combined with `tty:true`; use `process write` instead. Initial `cols` and `rows`
default to 80 by 24 and accept values from 1 to 1000.
Terminal stdout includes the child's stdout and stderr together; the result's
stderr stream is reserved for terminal bridge diagnostics. Windows uses ConPTY
on Windows 10 or later. On Unix, Jecode compiles its owned POSIX helper with an
installed system `cc` on first PTY use. Cleanup enumerates `/proc` on Linux and
uses `/bin/ps` on macOS.
The helper is cached afterwards.

Without `tty`, `wait_ms`, timeouts, saved output and previews follow the Bash
contract above. `protect` uses the same declared workspace paths as Bash; a
receipt describes the prelaunch snapshot, not command success.

## Persistent processes

Pass `wait_ms` to `bash` or non-terminal `exec` for a server, watcher or long
build. Jecode waits up to
that many milliseconds and returns a `process_id` plus status; `wait_ms: 0`
returns immediately. Expiring the wait does not stop the process. Bash and
non-terminal exec have no later stdin input; terminal exec accepts input through
`process`. An optional `timeout_seconds` remains a separate deadline from launch.

Tool results describe the state when the call returned. The TUI labels a Bash
launch `Started in background` and a running Process check `Running at last
check`; these cards are snapshots. Use `/terminal` for the current active list.

`process` actions are:

| Action | Behavior |
| --- | --- |
| `list` | Returns IDs, commands, statuses and output references for this conversation. Use it to recover an ID after compaction. |
| `poll` | With `process_id`, returns current status and new output since the previous result; optional `wait_ms` defaults to 0. |
| `stop` | With `process_id`, requests termination of the process and descendants and waits up to 10 seconds. If the result is `stopping`, poll again to confirm. |
| `write` | Sends `text` or one `key` to a terminal process. Supported keys: Enter, Tab, Escape, Backspace, Up, Down, Left, Right, Home, End, Delete, Ctrl+C, Ctrl+D and Ctrl+Z. At most 64 KiB of input per call. |
| `resize` | Changes a terminal process to `cols` by `rows`, each from 1 to 1000. |
| `interrupt` | Sends the terminal interrupt character. Unix follows the current `ISIG`, `VINTR` and `NOFLSH` settings; raw mode receives byte `0x03`. Windows sends Ctrl+C through ConPTY. `write` with `key:"Ctrl+C"` uses the same operation. |

A `write` receipt confirms delivery to the terminal control pipe, not completion
of the child's command. If delivery is unconfirmed, the input may still have
arrived or remain queued; the timeout does not close the channel. Resize and
interrupt use a separate control connection so bridge input backpressure on Unix
cannot block control requests. When terminal signal processing flushes input,
older queued input is discarded, including bytes still arriving from the writer;
input submitted after the interrupt is retained. `NOFLSH` preserves queued input.
In raw mode, the interrupt remains input data and requires the application to read it.
If the terminal has no foreground process group, the interrupt is consumed as
a no-op; it does not hold up later input or resize requests.
Inspect process output and workspace state before retrying. `protect`
on `write`, `resize` or `interrupt` captures preimages for that call's declared
workspace paths before sending input or control.

Statuses are `running`, `stopping`, `exited`, `timed_out`, `stopped` and
`unknown`. Only a finished `exited` process has an `exit_code` (possibly
`null`). `unknown` means completion or cleanup could not be confirmed; inspect
the workspace before retrying. An empty output preview says nothing about
readiness or completion: check `status`.

Each result reads the next 16 KiB of saved output per stream by default;
`output_limit_bytes` changes this budget, capped at 256 KiB, for the initial
command result or a `process poll`/`stop`. A budget smaller than one UTF-8
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
its stop. For an interactive terminal in the TUI, press `i` to send keys
directly; Ctrl+] leaves input mode. These keystrokes do not enter model history.
In plain chat, `/terminal` lists active processes and `/terminal ID` shows
status and the last 64 KiB of each stream. A stop request can remain
pending; check again for its final status.

For connection retries and context compaction during long turns, see
[Long-running work](LONG_WORK.md).
