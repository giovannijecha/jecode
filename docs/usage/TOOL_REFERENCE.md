# Tool reference

Exact arguments, defaults and limits of the tools Jecode gives the model. For an
overview, read [Tools](TOOLS.md) first. The model receives these contracts as
tool schemas; this page is for understanding results, diagnosing behavior and
writing [skills](SKILLS.md).

## Arguments

Every tool except `steps` accepts an optional boolean `parallel`. The routine tools `read`,
`list`, `search`, `map`, `bash`, `exec`, `process`, `web`, `browser`, `mcp` and `lsp` also
accept two display-only strings: `work`, a short label that groups related calls
into a [Work chapter](../tui/TOOL_ACTIVITY.md#work-chapters), and
`description`, a caption for the call's card. `agent` accepts `work` too.
Neither changes what the call does.

| Tool | Required | Optional |
| --- | --- | --- |
| `read` | `path` or `paths` | `offset`, `limit`, `byte_offset` (with `path` only) |
| `write` | `path`, `content` | — |
| `edit` | `path` and `edits`, or `files` | Top-level `old_text` and `new_text` instead of `edits`, for compatibility |
| `list` | — | `path`, `recursive`, `pattern`, `include_hidden`, `include_ignored`, `offset`, `limit` |
| `search` | `query` | `path`, `pattern`, `case_sensitive`, `context`, `include_hidden`, `include_ignored`, `offset`, `limit` |
| `map` | — | `action`, `path`, `query`, `pattern`, `offset`, `limit`, `include_hidden`, `include_ignored`, `id`, `title`, `summary`, `sources` |
| `bash` | `command` | `protect`, `timeout_seconds`, `wait_ms`, `output_limit_bytes` |
| `exec` | `executable` | `args`, `cwd`, `env`, `stdin`, `tty`, `cols`, `rows`, `protect`, `wait_ms`, `timeout_seconds`, `output_limit_bytes` |
| `process` | `action` | `process_id`, `text`, `key`, `cols`, `rows`, `protect`, `wait_ms`, `output_limit_bytes`, `output_mode` |
| `checkpoint` | `action` | `ref`, `paths`, `expected`, `preview`, `limit` |
| `web` | `action`; `url` for `fetch` and `download`; `urls` or `crawl_id` for `crawl` | `method`, `headers`, `body`, `timeout_seconds`, `max_bytes`, `max_pages`, `max_depth`, `same_origin` |
| `browser` | `action`; `url` for `navigate`; `key` for `press`; `text` for `fill`/`type`; `expression` for `evaluate`; `method` for `cdp` | `url`, `visible`, `profile`, `browser_id`, `tab_id`, `selector`, `text`, `key`, `modifiers`, `params`, `timeout_ms`, `x`, `y`, `delta_y`, `width`, `height`, `filter`, `limit`, `cursor` |
| `mcp` | `action`: `list`, `tools` or `call`; `connection` for `tools` and `call`; `tool` for `call` | `arguments` (JSON object), `query`, `offset`, `limit` |
| `lsp` | `action`: `status`, `definition`, `references` or `diagnostics`; `path` except for status; `line` and `column` for positional actions | `server`, `include_declaration` |
| `agent` | `action` | See [Collaborating agents](COLLABORATION.md#agent-actions). |
| `ask_user` | `questions` | See [Questions and plans](QUESTIONS_AND_PLANS.md#question-format). |
| `todo_list` | `action` | See [Questions and plans](QUESTIONS_AND_PLANS.md#plan-format). |
| `steps` | `steps` | See [below](#steps). |

## Steps

`steps` runs prepared tools in order with one call, so the model does not pay a
request between calls that do not need each other's results. Each entry has
`tool` and `arguments`, the same arguments as that tool alone, and keeps its own
permission check, checkpoint and card. There is no fixed entry-count limit.
The schema lists the tools available as steps. `ask_user` needs its own call;
nested `steps`, unknown tools and empty arrays are refused before anything runs.

The call has one result, a `steps` array in the same order. Each entry names its
`tool` and has its `result`, or `outcome: "not_run"` after a failed step. A step
fails on an `error`, a refused or unknown `outcome`, a timeout, a cancellation, a
non-zero exit, a `checkpoint` restore with conflicting, failed or skipped paths
or, for `edit`, any file that failed; a multi-file `read` reports each path and goes on.
A successful MCP `completed` outcome continues. LSP
statuses other than `ready` and background command timeouts stop the rest.
A restore's reported skips also include partial-capture warnings; those stop
the prepared sequence too, leaving the complete result for the model to assess.
`ask_user` and `steps` cannot be steps. Media files need a `read` of their own,
because the model is shown them only after a single read. If Jecode closes during
the call, its outcome is unknown and the model is told to check before repeating
it.

## Scheduling

These calls run in parallel automatically when adjacent, up to four at a time:
`list`, `search`, single-path `read` (except `history:` and `skill:` references),
`map` overview, `checkpoint list` and `inspect`, `process list`, `mcp list` and
`tools`, and `web fetch` or `download` using GET or HEAD without a body.

`parallel: true` on adjacent calls declares them independent, including writes
and commands; they run in waves of at most four. A call without it ends the
declared group. `parallel: false` always runs in order. `agent`, `ask_user`,
`steps`, `todo_list`, multi-path `read`, `history:` and `skill:` reads and calls
that need approval always run in order. Jecode never infers independence from different paths.

In a declared group, each result is saved as it completes and shown in the TUI;
the model receives all results in the original order. One failure does not undo
another call. If Jecode stops mid-group, completed calls keep their results,
calls that had started have an `unknown` outcome and calls that had not started
are marked `not executed`. Nothing is replayed.

## read

| Argument | Default | Notes |
| --- | --- | --- |
| `offset` | 1 | First line, 1-based. |
| `limit` | 200 | Lines per page, at most 2,000. |
| `byte_offset` | — | Zero-based byte position; overrides `offset` and returns unnumbered text with a byte range. |

Each page is also capped at 64 KiB. Continue with `next_offset`; for a line
split by the byte cap, pass `next_byte_offset` as `byte_offset`. `total_lines`
appears at the end of the file. These are page limits, not a file-size limit,
and paging does not apply to images or PDFs.

Instead of `path`, `paths` reads up to 12 files in one call (not both). Each
file uses the default window and the results come back in order, each with its
own result or error, so one missing file does not stop the others. Paths
outside the working folder fail and images or PDFs are not shown: read them
alone with `path`, which also asks for access when needed. Jecode starts no new
read once the combined result reaches 96 KiB, and later paths report that they
were not read. The read that crosses that size is kept, so the result can exceed
96 KiB.

Images and PDFs are recognized from their contents. The result returns an
`attachment:ID` reference to a stored copy, which stays readable after the
original changes or the temporary area is cleared. Other binary files and
invalid UTF-8 fail.

`path` also accepts these references:

| Reference | Content |
| --- | --- |
| `tmp:relative/path` | A file in the conversation's [temporary area](TEMPORARY.md). Also accepted by `write` and `edit`. A read that finds nothing there falls back to the areas of the agents above, or for a main agent of its collaborators; if several match, the error names them. |
| `tmp:AGENT:relative/path` | A file in one collaborator's temporary area, for example `tmp:a2:report.txt`. Read-only. |
| `output:SESSION_ID:OUTPUT_ID:stdout` or `:stderr` | A command's complete saved stream, including one saved by the main agent or, for a main agent, by its collaborators. |
| `history:N` | The original message at zero-based index `N`. |
| `history:requests` | All your original requests, in order. |
| `history:memory` | The current compaction summary. |
| `history:processes` | Saved outcomes of finished processes, one JSON object per line. |
| `history:processes` | Recorded process outcomes, including results saved before the model received them. Reading them does not restart a process. |
| `skill:NAME`, `skill:NAME/path` | A skill's `SKILL.md`, or a file inside the skill. |
| `attachment:ID` | An attached file; images and PDFs are shown again. |
| `checkpoint:rc-ID:path` | A file as saved before one modifying call. |

History pages survive compaction and resume, exclude provider-specific
reasoning, and mask known credentials. Input you took back into the composer is
excluded. Only `tmp:relative/path` can be written; the others are read-only.

## write and edit

`write` creates or replaces a complete UTF-8 file and returns `bytes_written`.
`edit` takes `edits: [{"old_text": "...", "new_text": "..."}, ...]`, applies
them in order and installs the result once. A top-level `old_text` and
`new_text` pair is also accepted for compatibility; supplying both forms with
content is an error.

To change several files in one call, send `files: [{"path": "...", "edits":
[...]}, ...]` instead of `path` (up to 12 entries, each file once, not combined
with top-level `edits`, `old_text` or `new_text`; a top-level `path` beside `files`
is ignored). Each file is applied all or nothing like a single
`edit`; a failed file does not stop the others, and the result lists each file
in order with its own outcome or error. The call needs one permission decision.

Successful edits report `changed`: `true` when a replacement was installed,
`false` when none was needed. Emptying a file still reports `true` even though
`bytes_written` is zero.

An ambiguous edit returns two bounded literal `match_context` excerpts with
match byte offsets and `excerpt_byte_offset`. `match_context_source` says
`original` for the first replacement or `staged` after preceding replacements;
only `original` offsets apply to `read` with `byte_offset`, because staged text
is not on disk. Long matches also provide a bounded `after_text` excerpt at
`after_byte_offset`, so the text after the match remains available without
repeating the whole match. Use these excerpts to make the next exact match unique.
`current_content` still previews the unchanged original file; no replacement
was saved. For a missing match, read the required section before retrying.

`edit` requires UTF-8 text without NUL bytes. `write` can replace an existing
binary file with the supplied content. Both refuse read-only files and paths
outside the allowed roots. Both return a `checkpoint` receipt; see
[Checkpoints](CHECKPOINTS.md#what-is-saved).

Large edits keep the original file and the replacements as a virtual view, so
memory stays bounded regardless of file size; only per-edit range metadata grows
with the number of edits.

## list and search

Paths default to `.` and results are workspace-relative with `/` separators.
Absolute paths inside the project are accepted.

| | `list` | `search` |
| --- | --- | --- |
| Default page size | 200 entries | 100 matching lines |
| Maximum `limit` | 2,000 | 1,000 |
| Other defaults | `recursive: false` | `case_sensitive: true`, `context: 2` (0–10) |

- `pattern` is a path glob with `*`, `?`, `**` and bracket classes. Without `/`
  it matches the basename; with `/` it matches the workspace-relative path.
  Windows matching ignores ASCII case. Patterns filter results without pruning
  directories from a recursive walk.
- `query` is literal, non-empty, single-line text. `case_sensitive: false`
  ignores ASCII case only.
- Links and Windows junctions are listed but never followed.
- An explicitly selected path bypasses its own hidden or ignored status; its
  contents are still filtered.
- Results are capped at 64 KiB per page. Continue with `next_offset` and the
  same arguments until it is `null`; restart after the workspace changes.
- Each match has `line`, `text`, `byte_offset` and `before`/`after` context. Long
  lines are searched in chunks and previewed around the match, with
  `text_truncated`; use `read` with the `byte_offset` to see the full text.
- Binary and non-UTF-8 files are counted in `skipped`. Unreadable paths appear in
  `errors`. `complete` is `false` when more results exist or errors prevented a
  full search, so an empty result proves absence only within the readable scope.

## map

`action` is `overview` (default), `remember` or `forget`.

**overview** accepts `path`, `query` (case-insensitive literal text in paths,
titles, summaries and roles), `pattern`, `include_hidden`, `include_ignored`,
`offset` (default 1) and `limit` (default 100, at most 1,000). Entries are
`area`, `landmark`, `symbol` or `note`, with an origin of `filesystem`,
`source` or `model`. Collection stops at 100,000 candidate entries, keeps 2,000
rows, reads up to 80 file headers of 8 KiB, and reads at most 4 MiB for a
focused file outline. `inventory_truncated` means the path or query must be
narrowed; paging cannot recover it.

**remember** saves a note:

```json
{"action":"remember","title":"Configuration entry point","summary":"Setup reads the user-scoped config here.","sources":[{"path":"src/config.rs","line":1}]}
```

`title`, `summary` and at least one source are required; `line` is optional and
1-based. The result returns an `id`; pass it to `remember` to update the note or
to `{"action":"forget","id":"..."}` to remove it. A note holds up to 16 sources,
a 200-byte title and an 8 KiB summary. A project holds up to 256 notes and
8 MiB.

Freshness is `current` (the cited files are unchanged), `changed`, `missing` or
`unverified` (Jecode could not check, for example because the page's sources
exceed the 64 MiB checking budget). Unchanged sources do not prove a note is
still correct.

## bash and exec

| Argument | Behavior |
| --- | --- |
| `timeout_seconds` | Positive integer measured from launch. Without it there is no timeout. |
| `wait_ms` | Wait this many milliseconds, then return a `process_id` and the next output window, leaving the command running. `0` returns at once. Omitted, the call waits up to 10 seconds and returns the result below if the command ends in time. |
| `output_limit_bytes` | Preview size per stream, default 16 KiB, at most 256 KiB. Larger values are clamped. |
| `protect` | Files or directories to save in a checkpoint before launch. `"."` protects the whole eligible workspace. |

A command that ends within an omitted `wait_ms` returns `exit_code`, `cancelled`,
`timed_out`, byte counts and a preview of each stream; truncated previews keep the
beginning and end. `stdout_file` and `stderr_file` reference the complete saved
streams. Known credentials are masked before output is saved. If saving output or
confirming cleanup fails after launch, the result is `outcome: "unknown"`.

One still running after those 10 seconds returns `yielded: true` and the fields of
a `process` result, and keeps running. Interrupting the turn during that wait stops
the command; interrupting a wait the model chose returns `wait_cancelled: true` and
leaves it running. `moved_to_background: true` means the user released the wait
from `/process`. In a `steps` call, a command that yielded stops the remaining steps,
which were planned on its outcome; a step with its own `wait_ms` does not.

**bash** runs the [command language](SHELL.md) with closed stdin and a fresh
state per call. Its environment drops `BASH_ENV`, `ENV` and provider
credentials and sets `TERM=dumb`.

**exec** runs `executable` with `args` passed literally: no shell parsing,
quoting or expansion. Details:

- A bare name is found through `PATH`. On Windows only the `PATH` entries are
  searched, in order; a name without a dot is tried as `name.exe`, and a dotted
  name such as `python3.11` is tried as written before adding `.exe`.
- `cwd` is absolute or relative to the project; an outside directory needs
  approval in Workspace. An executable path with a directory resolves against
  `cwd`.
- `env` maps names to strings to set, or to `null` to remove. Provider
  credentials are removed first.
- `stdin` sends a UTF-8 string and closes the stream; without it, stdin is
  closed at once.
- `tty: true` starts an interactive terminal and always returns a
  `process_id`. Its stdout combines the program's stdout and stderr. `cols` and
  `rows` default to 80 by 24 (1–1000). `stdin` cannot be combined with `tty`;
  use `process write`. Requires Full.

## process

| Action | Behavior |
| --- | --- |
| `list` | IDs, commands up to 200 characters, statuses, elapsed seconds and output references for this conversation's running and recent processes. The oldest finished ones leave the list once the model has seen them; 32 are kept. |
| `poll` | New output and status for `process_id`. Optional `wait_ms`, default 0. |
| `stop` | Stop the process and its descendants, waiting up to 10 seconds. If the status is `stopping`, poll again. |
| `write` | Send `text`, or one `key`, to an interactive process. Keys: Enter, Tab, Escape, Backspace, Up, Down, Left, Right, Home, End, Delete, Ctrl+C, Ctrl+D, Ctrl+Z. At most 64 KiB per call. |
| `resize` | Resize an interactive process to `cols` by `rows` (1–1000). |
| `interrupt` | Send the terminal interrupt, as Ctrl+C would. |

Statuses are `running`, `stopping`, `exited`, `timed_out`, `stopped` and
`unknown`. Only `exited` has an `exit_code`. `unknown` means completion or
cleanup could not be confirmed. An empty output preview says nothing about
readiness; check `status`.

Each result returns the next window of saved output per stream, 16 KiB by
default (`output_limit_bytes` up to 256 KiB). `*_offset`, `*_next_offset` and
`*_remaining_bytes` describe the window. The default `output_mode: "next"` never
skips unread output. `output_mode: "tail"` on `poll` or `stop` returns the
newest bytes instead and reports skipped bytes in `*_skipped_bytes`; the full
log stays readable.

When no call is waiting for a process that ends, its outcome joins the session
state of the next accepted request, once: identity, status, exit code when known,
elapsed time, command, `stdout_file`, `stderr_file` and up to 1 KiB from the end of
each stream, credential-masked. It moves no output cursor, so `poll` still returns
everything unread. A `poll`, `stop` or `list` that reports the final status replaces
this delivery. An idle main conversation continues when a process ends by itself,
unless the turn was stopped or failed, or queued input or an open menu, question or
permission prompt comes first; the environment's `background_resume` is `automatic`
for idle continuation, `on_input` when plain chat must wait for another entered line,
and `none` for single prompts and collaborators, whose answer ends the run even if a
process ended while it was written. With `on_input` or `none`, the model waits for an
outcome needed by its answer rather than promising a follow-up that cannot start by
itself. Saved outcomes stay readable through `history:processes`, and a compaction
keeps an outcome that has left the window in its summary.

A `write` receipt confirms delivery to the terminal, not that the program
processed it. On Unix, `interrupt` follows the terminal's current `ISIG`,
`VINTR` and `NOFLSH` settings; in raw mode the program receives byte `0x03`. On
Windows it sends Ctrl+C through ConPTY. `protect` on `write`, `resize` or
`interrupt` saves the declared paths first. Interrupting a `poll` wait leaves the
process running.

## web

| Action | Behavior |
| --- | --- |
| `fetch` | Returns status, final URL, MIME type, size, a text preview and up to 100 links, with paths to the raw response (`tmp:web/`), the extracted text and the full link list. Text extraction reads at most the first 4 MiB. |
| `download` | Saves the raw response and returns its path and metadata, without extraction. |
| `crawl` | Starts from `urls`, or continues a previous crawl with `crawl_id`. Returns up to 20 page summaries and a `manifest_path` (JSON lines) with every fetched page. |

- Requests use GET, or POST when `body` is given without `method`. HEAD saves
  only the response headers. `headers` values are strings.
- `max_bytes` fails the request when the response is larger; without it there is
  no size limit.
- Redirects: POST becomes GET after 301, 302 or 303; 307 and 308 keep the method
  and body. Custom headers follow redirects only within the same origin.
- `crawl` uses GET only. Defaults: `same_origin: true`, `max_depth: 1`,
  `max_pages: 20` per call. Headers are sent only to the starting origins. The
  crawl's headers, timeout and size limit are saved and reused when continuing.

## browser

Actions: `open`, `tabs`, `navigate`, `snapshot`, `click`, `fill`, `press`, `type`,
`scroll`, `wait`, `screenshot`, `evaluate`, `cdp`, `logs`, `close`, `stop`.

- `open` starts a browser, headless unless `visible: true`, and returns
  `browser_id` and `tab_id`. `profile` selects a named persistent profile.
  Without `browser_id`, the newest browser is used; without `tab_id`, an
  available page tab.
- `width` and `height` (both or neither, 1–10,000 CSS pixels) set the page
  viewport on `open` or before a `screenshot`, and stay with the tab.
- `snapshot` returns page text (up to 12,000 characters) and up to 100
  interactive elements, each with a reference such as `@1`, its role,
  accessible name, type, current value (never for passwords) and state. The full
  snapshot is saved to `snapshot_path`. `selector` accepts a reference or a CSS
  selector.
- `press` sends key-down and key-up events for a character, a named key such as
  `Enter` or `ArrowLeft`, or a chord such as `Ctrl+z`, `Meta+z` or `Shift+Tab`.
  `modifiers` is an optional CDP bitmask: Alt = 1, Ctrl = 2, Meta/Command = 4,
  Shift = 8; combine flags by adding them. Chord flags and the mask combine.
  Key and modifier names ignore case; `Left`, `Up`, `Right`, `Down`, `Esc`, `Del`
  and `Option` are aliases. Printable keys insert text; shortcuts do not insert
  their letter. ASCII physical codes use the US keyboard layout; Unicode characters
  preserve their text without inventing a physical code.
  On macOS, Command+A/C/X/V/Z and Command+Shift+Z include the corresponding
  native editing command; page key handlers can still cancel its default action.
- `type` sends `text` as successive key-down/key-up pairs. Newlines press
  `Enter` (CRLF counts once), and tabs press `Tab`. For `press` and `type`, an
  optional `selector` focuses the element once; subsequent input follows page
  focus, including navigation performed by the app. An element that neither takes
  focus nor contains it is an error. The whole keyboard action shares `timeout_ms`
  (default 15,000). Keys already delivered stay applied, so an error after a partial
  sequence says how many key-down/key-up pairs completed. If key-up fails, the
  current key may already have acted; inspect the page before retrying.
  A key that navigates replaces the page later keys
  would reach: send those in a new call after `wait`.
- `fill` replaces an input's value using text insertion, without per-character
  keyboard events. Use `type` and `press` when verifying an editor, shortcuts or
  keyboard navigation. For example, after clicking cell A1,
  `{"action":"type","text":"first\nsecond\tthird"}` types `first`, presses
  Enter, types `second`, presses Tab, then types `third` wherever focus moved.
- A JavaScript dialog (`alert`, `confirm`, `prompt`, `beforeunload`) blocks its page.
  The action that meets one fails at once with its type and text, and later actions
  on that tab fail the same way while it remains open. Use `cdp` with
  `Page.handleJavaScriptDialog` and `params` such as `{"accept":true}`, or close it
  in a visible window. Before refusing a later action, Jecode refreshes pending
  dialog events through a browser-side command, so an external close also resumes
  the tab. A second dialog remains pending until it is answered in turn.
- `wait` waits for `text` in the page or the selected element, up to
  `timeout_ms` (default 15,000). On timeout it reports the elapsed time, tab,
  URL and whether the selector or the text was missing.
- `scroll` takes `delta_y` and optional `x` and `y`. `evaluate` runs a
  JavaScript `expression`. `cdp` sends a raw DevTools `method` with `params`.
- `screenshot` saves an image under `tmp:browser/` and shows it to the model on
  the next request, with an `attachment:ID` that outlives temporary cleanup.
- `logs` pages through the tab's console, exception and network events, kept
  from Jecode's first connection to the tab (newest 1,000). `filter` is
  `default`, `console`, `errors`, `network` or `all`; `limit` defaults to 50
  (at most 200). Pass `next_cursor` as `cursor` to continue. `missed` counts
  evicted events and a `gap` entry marks a reconnection. A page can hold fewer
  entries than `limit`, even zero, while `more` is true.
- `close` closes a tab; `stop` ends the browser.

## mcp

`mcp list` shows configured connections with their stable IDs, names, enabled
state and endpoint origin (scheme and host only). `mcp tools` discovers remote
tool names and exact input schemas, optionally filtered by `query`; `offset`
defaults to 0 and `limit` to 8 (1–50). `mcp call` forwards `tool` and
`arguments` (default `{}`; a JSON string that encodes an object is also
accepted) to the selected enabled connection.
An oversized discovery page returns `truncated: true`, a `preview` and a
`result_file` containing the complete page and exact schemas. Read that file
before constructing arguments; the inline preview is not a schema definition.

Structured and other MCP result blocks remain in the JSON result. Image, audio
and blob data are replaced by a placeholder with the MIME type and size. A
result that still exceeds 64 KiB keeps a text preview and `truncated: true`;
in both cases `result_file` is an `output:` reference to the full result,
readable with `read`. A remote `isError` is a tool failure; `outcome: unknown`
means the service may already have acted. Never infer rollback from local
checkpoints. See [MCP](MCP.md).
An interruption before dispatch returns `outcome: not_executed` and is displayed
as Cancelled; no remote call was sent.
