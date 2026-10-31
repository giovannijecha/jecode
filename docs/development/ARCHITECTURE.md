# Architecture

Jecode is one Cargo package built from the Rust standard library and original
project code, with no external crates. The agent loop owns conversation state
and tool execution. The TUI, plain chat and single-prompt CLI consume the
agent's events; terminal rendering never runs inside the agent loop.

This page is for contributors. For user-facing behavior, follow the links to
the usage guides.

## Module map

Paths are relative to `src/`.

| Area | Modules |
| --- | --- |
| Entry and setup | `main.rs`, `cli.rs`, `bootstrap.rs`, `bootstrap/`, `setup/` |
| Settings, credentials and effort | `config/`, `auth/accounts.rs`, `auth/openai/`, `auth/anthropic/`, `effort.rs`, `identity.rs` |
| Agent loop, conversation and context | `agent.rs`, `agent/`, `conversation/`, `context.rs`, `context/`, `events.rs`, `events/` |
| Input during a turn | `inbox.rs`, `inbox/`, `input.rs`, `agent/input.rs` |
| Questions and plans | `ask.rs`, `ask/`, `todo.rs`, `todo/`, `agent/ask/`, `agent/todo/` |
| Collaborators | `collaboration/`, `agent/collaboration.rs`, `agent/continuation.rs` |
| Permissions and command confinement | `permissions.rs`, `permissions/`, `agent/permission.rs`, `sandbox.rs`, `sandbox/` |
| Tools and their output | `tools/`, `file_change.rs`, `file_change/`, `output.rs`, `output/` |
| Command language and processes | `shell.rs`, `shell/`, `process.rs`, `process/`, `pty.rs`, `pty/` |
| File discovery and path filtering | `tools/discovery/`, `path_filter.rs`, `path_filter/` |
| Project map | `project_map/` |
| Web and browser | `http.rs`, `http/`, `web/`, `browser/` |
| Personal skills | `skills.rs`, `skills/`, `agent/skills.rs` |
| Saved conversations | `sessions/`, `agent/persistence.rs` |
| Workspace checkpoints | `checkpoints/`, `agent/checkpoints.rs`, `session/checkpoints.rs` |
| Temporary working files | `scratch/`, `agent/temporary.rs` |
| Attachments, clipboard, copy and export | `attachments/`, `tools/attachment.rs`, `clipboard.rs`, `clipboard/`, `copy.rs`, `copy/`, `export.rs`, `export/` |
| Model backends | `backend/`, `backend/openrouter/`, `backend/openai/`, `backend/anthropic/` |
| Native adapters | `native.rs`, `native/`, `disk.rs`, `disk/` |
| Interfaces | `session.rs`, `session/`, `console.rs`, `tui/` |
| Shared utilities | `json.rs`, `cancel.rs`, `redact.rs`, `utf8.rs` |

## Request lifecycle

1. `bootstrap.rs` resolves configuration, the working directory and the
   interface. An interactive terminal on stdin and stdout selects the TUI; a
   prompt argument, piped stdin or `--plain` selects the line-based CLI.
2. Before recording a new request, the agent snapshots every `AGENTS.md` in the
   launch directory and its real subdirectories. Each file keeps its directory
   scope, and the snapshot stays fixed for the whole turn, including after
   compaction. Parent directories are never read.
3. The context layer projects the active conversation. The selected `Backend`
   translates that canonical transcript and the neutral tool catalog into its
   wire format and streams text, reasoning progress and tool calls back as agent
   events.
4. Tool calls execute only after the complete model response arrives. The agent
   saves the call boundary, runs the calls and records their results, then sends
   the next request. Queued user messages from the shared inbox join before that
   request.
5. Completion, cancellation or failure saves the conversation. The interface
   renders the outcome and keeps unsent input.

The harness supplies tools, context and history. It adds no completion gates,
repeated-command refusals or validation of model output.

### Tool scheduling

Up to four adjacent eligible reads run together automatically: `list`,
`search`, `read` (except `history:` and `skill:` references), `map overview`,
`checkpoint list` and `inspect`, `process list`, and `web fetch` or `download`
using GET or HEAD without a body.

A model can declare adjacent calls independent with `parallel: true`, including
writes and commands. Declared groups run in waves of at most four. `parallel:
false` forces serial execution, and omitted modifying calls stay serial.
Declared and automatic groups never merge, and the agent never infers
independence from paths. `agent`, `ask_user` and `todo_list` calls are always
serial boundaries.

### Streaming and persistence

Backends deliver text as deltas. The agent masks credentials incrementally,
even when a secret is split across deltas, and produces display and persistence
snapshots at terminal cadence. The first visible prefix is saved synchronously;
later partial snapshots use background autosave so the stream observer never
waits on disk. Completion and tool boundaries save synchronously.

The TUI worker keeps only the latest streaming snapshot, so transient indicators
never block on a full queue. Significant events use a bounded ordered queue with
cancellation-aware backpressure.

### File changes and Work chapters

`write` and `edit` capture a contextual change at execution time.
`file_change/` owns text alignment and serializable hunks, independently of the
terminal. A local `tool_change` event is saved beside the canonical tool result
and replayed to the TUI. It never reaches provider messages or context
estimates, and the model-facing result is unchanged. Capture is bounded: an
unavailable diff never rejects a valid write or edit. Local `turn_start` events
keep execution-turn grouping across queued follow-ups and resume.

The optional `work` argument stays in each tool call's canonical arguments and
is replayed through every provider's native function-call shape.
`tui/state/rows.rs` forms a Work chapter from two or more contiguous eligible
calls with the same normalized label and execution turn (`Presentation.turn_id`).
`tui/cards/chapter.rs` derives the chapter's facts and status from its members.
See [Tool activity](../tui/TOOL_ACTIVITY.md) for the user-visible result.

### Project map

`project_map/survey.rs` reuses discovery's filtered walker for directory areas
and candidate landmarks. A focused Rust or Markdown file adds lexical
declarations or headings with line references; this is not compiler or
dependency analysis. Collection is bounded and reports omitted inventory.

`map remember` stores model-written notes with source paths, optional line
excerpts and full-file fingerprints. `project_map/sources.rs` compares the
fingerprints for the requested page and reports `current`, `changed`, `missing`
or `unverified`. Notes live at
`~/.jecode/projects/<canonical-root-key>/map.json`, independent of sessions.
Writes lock, reload and atomically replace that file. Neither startup nor the
system message scans the project or injects the map.

### Context and compaction

`context.rs` derives the input budget from the **Auto-compact at** percentage
and the effective model window, lowered when a provider rejection reveals a
smaller ceiling. Prompt estimates calibrate from provider usage when it is
available.

Before summarizing, `agent/compaction.rs` turns large tool payloads into
previews while keeping the original results. If that is not enough,
`agent/summary.rs` asks the same model, at medium effort when supported, for a
plain-text handoff targeting at most 16 KiB. The transcript is folded into the
previous summary in chronological portions. A verbatim tail of recent messages
is kept within a quarter of the input budget; a tool call and all its results
stay on the same side of that boundary. Failed compaction leaves the previous
context in place.

`context/view.rs` builds a stable archived-request prefix at each compaction,
keeping the first and the latest user requests within its byte budget.
`agent/history.rs` exposes original messages to the model through
`read history:N`.

The built-in instructions live in `agent/PROMPT.md` and are compiled into the
executable. Project instructions and the skill catalog form the stable system
prefix. Permissions, temporary paths, plans and collaboration state form a
separate final request-only message, so changes to them do not invalidate the
cached prefix. Neither projection rewrites the saved system message.

Each request attempt records its operation, resolved effort, timing and the
usage counters the provider reported. Background title requests run at low
effort and compaction at medium; both fall back to the provider default when
the model does not advertise that level.

### Input and processes

The TUI and agent share a locked inbox. Accepting a follow-up into the active
turn and removing it from the saved queue happen in one save. Local commands
wait until the interface is idle.

`process::start_logged` creates a command handle; `Started::supervise` owns its
timeout, cancellation, output and cleanup. Foreground and persistent commands
share that path. Each persistent process has a monitor thread and a stop token
in the conversation's `tools/processes.rs` registry. `/terminal` observes the
registry through a weak handle with its own output cursors, so it cannot keep a
process alive. Conversation switches, deletion of the live session and agent
drop stop and join registered processes.

## Questions and plans

`ask::Observer` connects the terminal's question surface to the main worker
without putting rendering in the agent loop. A `user_question` journal event
names the original tool call, and submitting or dismissing appends a durable
`user_answer` before the waiting worker consumes it. Collaborators and headless
runs receive an unavailable result.

Each successful `todo_list set` result contains the whole numbered plan; there is
no second persisted copy. `todo::latest` pairs results with their assistant
batch so reused provider call IDs cannot select an unrelated result. The request
environment projects the current plan from original history, including after
compaction.

User contract: [Questions and plans](../usage/QUESTIONS_AND_PLANS.md).

## Collaborating agents

The main loop intercepts the `agent` tool. `collaboration::Observer` owns live
records, append-only receipts and worker handles. Each collaborator runs an
ordinary `Agent` on its own thread with an independent backend, tool resources,
input queue and saved conversation. It receives focused instructions instead of
the parent conversation, and keeps the model, effort and routing it was created
with. Collaborator tool catalogs omit `agent` and `ask_user`, and dispatch
rejects delegation from a collaborator independently of the provider.

Assignment updates are versioned and delivered through the input queue at model
boundaries. Sent and received receipts are separate. Review is recorded
explicitly by the main agent against the latest returned version; a textual
completion never changes collaborator state.

Receipts append to the parent session even while the main model is idle. Each
receipt records where it belongs (`collaboration/placement.rs`): the `work` label
of the agent call that wrote it, and whether a turn or call was running. The TUI
places receipts from those facts, not from arrival time, so live and resumed
views match. A new return while the main agent is idle starts
`Agent::continue_collaboration` without a synthetic user message; queued input
wins, and an explicit stop suppresses the continuation.

Storage lives under the project bucket's `collaboration/<session-id>/`. Child
histories use a separate `child-home`, so the conversation picker cannot open
them as main sessions. Running work interrupted by a crash restores as
`unknown` and is never replayed.

Isolated work is copied into owned storage without Git
(`collaboration/workspace/copy.rs`). One walk reads each file once and writes
the same bytes to the copy and the baseline while fingerprinting them. `diff`
produces bounded evidence and a review token over baseline, child and current
states; `integrate` rechecks selected paths and reports per-path outcomes.
File isolation does not restrict commands or external effects.

User contract: [Collaborating agents](../usage/COLLABORATION.md).

## Session storage

`sessions/` stores conversations in buckets derived from the canonical project
directory, and loading validates the directory recorded in the journal.
Exclusive writer leases prevent two processes from updating the same session.
Summary caches speed up listing without replacing journal validation.

Session files are `SESSION_ID.jsonl` journals in the `jecode.session.journal`
format, version 3. Each synced, newline-terminated transaction appends messages
and events with any changed metadata and input. Streaming text uses UTF-8-safe
prefix and suffix updates. The journal calls these saves checkpoints; they are
unrelated to workspace checkpoints.

Parallel tool batches record call IDs in `pending.reads` or `pending.batch` and
save each completed result, timing and optional diff in `pending.completed`
before the interface shows it. When the batch ends, canonical results are
inserted in source order in one save. Recovery keeps saved completions, marks
started calls without a saved outcome as unknown, marks unstarted calls as not
executed and never replays a call.

| Condition | Recovery |
| --- | --- |
| Failed append | Roll back to the last acknowledged length; require reopening if rollback fails. |
| Torn final transaction | Preserve the incomplete bytes separately and recover the previous complete save. |
| Corrupt complete transaction | Report the error and leave the file unchanged. |
| Earlier format | Report it as unsupported and leave the file unchanged. |

An attachment draft created before any model was selected stores a null
`model`. Builds that require a model string cannot open such drafts.

User contract: [Saved conversations](../usage/SESSIONS.md).

## Workspace checkpoints

`checkpoints/` saves the state before one modifying tool call under
`~/.jecode/checkpoints/<project-name>-<canonical-root-id>/`. Each call writes one
immutable `rc-ID.rcp` pack: a header, raw file bytes, checksummed metadata with
offsets and a footer. `.internal/` holds `workspace.id`, `identity.lock` and
path leases. There is no object graph, global replay, after-state payload, delta
or compaction.

| Part | Responsibility |
| --- | --- |
| `store.rs`, `format.rs`, `capture.rs` | Immutable packs, streamed preimages, checked metadata, durable publication, readers and expiry. |
| `paths.rs`, `state.rs`, `files.rs` | Scope discovery, complete-directory proofs, path identity and confinement, staged installation. |
| `restore.rs`, `restore/` | Stateless inspection tokens, current-byte checks and restore with per-path undo. |
| `locks.rs`, `pressure.rs` | Cancellable path leases and advisory free-space admission. |
| `session.rs`, `actions.rs`, `show.rs`, `command.rs`, `render.rs` | Tool receipts, the three model actions, saved reads and local commands. |

File tools hold a lease from reading through installation. Commands capture
their declared `protect` paths before launch and then release the lease; no
work happens after the command finishes. Directory walks skip Git-ignored
directories using an owned `.gitignore` reader and record the rules they used.

Inspection hashes the selected current states into a stateless token. Restore
requires that token or an explicit `latest`, verifies saved content, captures an
undo and rechecks current bytes before each installation. Packs expire 24 hours
after creation; a maintenance worker sweeps once a minute while Jecode runs, and
offline commands sweep too.

User contract: [Checkpoints](../usage/CHECKPOINTS.md).

## Native boundaries

Rust launches owned native adapters through `std::process::Command`. The
adapters are compiled during the Cargo build (`build.rs`), embedded in the
executable and extracted to the Jecode home's `cache/native/` directory on
first use.
They call OS APIs directly; the Rust code uses no `unsafe` and no FFI.

| Boundary | Implementation |
| --- | --- |
| Windows processes | A C# supervisor creates commands inside Win32 Job Objects and reports completion through a private named pipe. A separate stop channel keeps cancellation independent of blocking reads. |
| Windows command confinement | Workspace commands run inside a Windows Process Security Environment with an OS-enforced filesystem policy; see [Permissions](../usage/PERMISSIONS.md#command-confinement). |
| Unix processes | A C guardian owns a process group, watches a private owner pipe and cleans up descendants on cancellation, timeout or owner loss. |
| Interactive terminals | Windows uses ConPTY with current-user named pipes; Unix uses POSIX PTYs with private Unix-domain sockets. Input and control use separate connections. |
| HTTPS and cryptography | Windows uses WinHTTP and .NET cryptography; macOS uses CFNetwork, Security and CommonCrypto; Linux uses sockets with the system libssl and libcrypto. Request bodies and tokens travel to the adapter on stdin, never in arguments or files. |
| Terminal input | Windows reads `CONIN$` through a C# adapter; Unix uses `termios` and `/dev/tty` with a guardian that restores terminal modes. |
| Clipboard and images | Windows PowerShell, macOS system commands, or the Linux X11/Wayland and GdkPixbuf adapter; see [Attachments](../usage/ATTACHMENTS.md#platform-requirements). |
| Disk space | `GetDiskFreeSpaceExW` on Windows and `statvfs` on Unix. |

Guardians stop owned descendants on cancellation, timeout or owner loss. Unix
children that create their own process group or session leave that cleanup
boundary. Losing the Unix owner and guardian at the same time can prevent
terminal restoration.

The TUI uses the alternate screen, 24-bit color and changed-row painting.
`--plain` bypasses the terminal adapter entirely. Native fixtures and host
checks are in [Platform support](PLATFORMS.md).

## Provider transport

`backend/` owns the inference contract: streamed text, flat tool calls, typed
usage including cached tokens, context limits, progress and failures. The agent
keeps its transcript, runs tools, compacts and saves without importing any
provider type. `conversation/` keeps portable message fields and tags native
extras (reasoning items, signed thinking, opaque output items) with the backend,
model and account that produced them. Only the matching adapter receives those
extras later; another backend gets portable history.

`auth/` holds account identity and credentials; `backend::Services` exposes
catalogs, credential checks and routing choices to setup and the TUI. Shared
redaction sources mask rotated tokens in tools, output readers and persistence
handles that already exist. Native helpers and tool subprocesses lose the
credential environment variables of every available backend before launch.

| Adapter | Route | Reference |
| --- | --- | --- |
| `backend/openrouter/` | OpenRouter chat completions with an API key, endpoint routing and PDF parsing | [Configuration](../usage/CONFIGURATION.md#openrouter-endpoint-routing) |
| `backend/openai/` | Stateless Responses requests with a ChatGPT account | [OpenAI backend](OPENAI.md) |
| `backend/anthropic/` | Streamed Messages requests with a Claude account | [Anthropic backend](ANTHROPIC.md) |

Caching follows each provider's model. Anthropic requests mark the stable system
block and recent durable user-message boundaries. Through OpenRouter, requests
to models that accept explicit cache markers (`backend/openrouter/cache.rs`)
mark up to four recent durable user or tool-batch boundaries. The ChatGPT route
relies on implicit caching with a stable per-conversation key. Reported cache
reads and writes stay separate from charged cost.

OpenRouter Auto routing keeps OpenRouter's fallbacks. With enough fresh endpoint
statistics, it adds soft preferences for throughput near or above half the
median and first-token latency near or below twice the median. A pinned endpoint
disables fallbacks and needs a compatible listing within its freshness window.

`agent/backend_tests.rs` drives two synthetic backends with different wire
layouts through tool execution, compaction, save, resume and a failed switch.
All three adapters have loopback fixtures for their HTTP and streaming paths.
