# Architecture

Jecode is one Cargo package built from the Rust standard library and original
project code. The agent owns conversation state and tool execution; CLI and TUI
consume its events. Terminal rendering stays outside the agent loop.

## Modules

Paths are relative to `src/`.

| Area | Modules |
| --- | --- |
| Entry and setup | `main.rs`, `cli.rs`, `bootstrap.rs`, `setup/` |
| Settings, credentials and model effort | `config/`, `auth/accounts.rs`, `auth/openai/`, `auth/anthropic/`, `effort.rs` |
| Agent, conversation and context | `agent.rs`, `agent/`, `conversation/`, `context.rs`, `context/`, `events.rs` |
| Input during a turn | `inbox.rs`, `agent/input.rs` |
| Collaborator workers, receipts and working copies | `collaboration/`, `agent/collaboration.rs`, `agent/continuation.rs` |
| Personal skills | `skills.rs`, `skills/`, `agent/skills.rs` |
| Tools and output | `tools/`, `file_change.rs`, `process.rs`, `process/`, `output.rs`, `output/` |
| Native file discovery and shared path filtering | `tools/discovery/`, `path_filter.rs`, `path_filter/` |
| Project orientation and shared source-backed notes | `project_map/` |
| Local web and browser | `web/`, `browser/` |
| Saved conversations | `sessions/`, `agent/persistence.rs` |
| Workspace checkpoints | `checkpoints/`, `agent/checkpoints.rs`, `session/checkpoints.rs` |
| Temporary working files | `scratch/`, `agent/temporary.rs` |
| Attachments | `attachments/`, `tools/attachment.rs` |
| Model backends | `backend/`, `backend/openrouter/`, `backend/openai/`, `backend/anthropic/` |
| Interfaces | `session.rs`, `session/`, `console.rs`, `tui/` |
| Shared utilities | `json.rs`, `cancel.rs`, `redact.rs`, `copy.rs`, `clipboard.rs`, `export.rs`, `disk.rs`, `disk/` |

## Request lifecycle

1. `bootstrap.rs` resolves configuration, working directory and execution mode.
   Interactive stdin/stdout selects the TUI; prompts, pipes and `--plain` use the CLI.
2. The agent snapshots `AGENTS.md` files in the launch directory and its real
   subdirectories before recording the request. Each file carries its directory
   scope; the combined snapshot applies throughout the turn, including after
   compaction. Parent directories are never searched.
3. The context layer projects the active conversation. The selected `Backend`
   translates that canonical transcript and neutral tool catalog into its wire
   request, then streams text and progress through agent events.
4. The agent saves tool boundaries, executes calls and records their results.
   Up to four adjacent eligible reads run together automatically after the
   complete model response: listings, searches, file reads, map overviews,
   checkpoint list/inspect, process list and web fetch/download using GET or
   HEAD without a body. History and skill reads remain ordered. An explicit
   `parallel: true` on adjacent calls declares independence, including for
   modifications; bounded waves keep at most four workers active. `false`
   forces serial execution, and omitted modifying calls remain serial.
   Explicit and automatic groups do not merge. The agent does not infer
   independence from paths. Follow-up messages from the shared inbox join
   before the next model request.
5. Completion, cancellation or failure checkpoints the conversation. The interface
   renders the result and keeps unsent input available.

The harness supplies tools, context and history. It adds no completion gates,
repeated-command refusals or model-output validation.

Backend updates carry text deltas. The agent masks split credentials incrementally
and produces display/persistence snapshots at terminal cadence, flushing the final
text on completion or failure. The first visible prefix saves synchronously;
later partial snapshots use background autosave to avoid periodic disk waits
in the stream observer. Completion and tool boundaries save synchronously.
The TUI worker keeps one latest streaming snapshot;
transient working indicators never wait for a full queue. Significant events use
a bounded ordered queue and retain cancellation-aware backpressure.

OpenRouter prepares uncached model capacities and stale endpoint metadata in
parallel, then validates a pinned endpoint and narrows the effective limits.
The stable instruction and conversation prefix is separate from the final
request-only session state. Anthropic requests mark the stable system block
and recent durable user-message boundaries for caching; OpenRouter Claude and
eligible OpenAI tool requests mark recent durable user/tool-batch boundaries.
OpenAI's ChatGPT route retains
implicit caching and the conversation's cache-affinity key; request-only state
is a final developer message, leaving the latest user/tool cache boundary stable.
Provider adapters consume the local state marker before transmission.
Reported cache reads and writes remain distinct from charged cost.
Provider inference latency and cache-hit rates need live measurements; loopback
and parsing tests establish local behavior only.
The `web` tool uses the build-embedded native HTTP adapter and keeps responses
and crawl state in the session temporary area. Web redirects are followed one hop at a
time under one request deadline, applying HTTP method changes and origin-scoped
headers. Crawls save GET request options with their queue for consistent resume.
The `browser` tool launches an installed Chromium-based executable and communicates
with its local DevTools
endpoint. Isolated profiles and browser artifacts use the session area; named
profiles use dedicated storage under the Jecode home. Both tools run locally,
independent of the selected model backend. Browser availability is a native host
boundary.

File writes and exact edits capture contextual changes at execution time.
`file_change.rs` owns their text alignment and serializable hunks, independently
of terminal rendering. A local `tool_change` event is saved beside its canonical
tool result and replayed to the TUI; it is excluded from provider messages and
context estimates. The model-facing tool result remains unchanged.
Optional capture bounds input size and alignment work; unavailable diffs never
reject an otherwise valid write or edit. Local `turn_start` events preserve
execution-turn grouping across queued follow-ups and resume.

The optional `work` argument on `read`, `list`, `search`, `map`, `bash`, `exec`,
`process`, `web`, `browser` and `agent`
stays in each tool call's canonical arguments. Both provider adapters replay it
through their native function-call wire shape, and saved sessions retain it
without a separate event or metadata envelope. `tui/state/rows.rs` forms a
chapter from two or more contiguous eligible calls with the same normalized label
and execution-turn ID (`Presentation.turn_id`); only map overview is eligible,
and of `agent` only its inspection actions and the events it receipts; a lone
event and `integrate` are not.
Text and other tools end the
group. Single eligible calls keep their normal cards. `tui/cards/chapter.rs`
derives aggregate facts and status from chapter members, omitting a facts row
that only repeats the homogeneous header status. It keeps every Bash target
visible in compact chapters. Missing labels, including older sessions, display
as Activity. Label normalization collapses whitespace; only the heading is
truncated to 60 terminal cells. Grouping compares the full normalized label;
the original saved arguments stay unchanged. The existing latest-turn `Ctrl+O`
state reveals complete individual cards inline.

### Project map

`project_map/survey.rs` uses discovery's filtered filesystem walker for live
directory areas and candidate landmarks. A focused Rust or Markdown file adds
lexical declarations or headings with line references; this is not compiler or
dependency analysis. Collection is bounded and reports omitted inventory.

`map remember` stores model-authored explanations with source paths, optional
line excerpts and full-file content fingerprints. `sources.rs` compares those
fingerprints on requested result pages, including edits outside Jecode, and
returns current, changed, missing or unverified status. Freshness covers the
referenced content; it cannot establish semantic correctness or account for
unreferenced dependencies.

`storage.rs` keeps notes at `~/.jecode/projects/<canonical-root-key>/map.json`,
separate from session deletion and backend state. Writes lock, reload and
atomically replace the project file so concurrent conversations preserve
independent notes. Checkpoints and map storage share original fingerprint,
file-lock and synced replacement helpers in `disk/`. Neither startup nor the
system message scans or injects the map. The tool's overview is eligible for
existing read batches and Work chapters; remember/forget execute in order
unless explicitly declared independent.

### Context and compaction

`context.rs` derives the input budget from the Auto-compact percentage and the
effective model window, lowered when a provider rejection reveals a smaller
ceiling. The trigger, result previews, retained requests and summary sizing use
that budget. Prompt estimates calibrate from provider usage when available.

Before model summarization, `agent/compaction.rs` tries native previews of large
tool payloads, including file-read content, while retaining complete original
results. The same reduction is attempted after a provider context rejection.
It keeps a verbatim tail within one quarter of the input
budget. An assistant tool call and all its results stay on the same side of the
boundary; an oversized group can leave no verbatim tail.
`agent/summary.rs` asks the same model at medium effort when supported for a
plain-text handoff targeting at most 16 KiB. This is a prompt target, not an
output truncation.
The initial output allowance is 2,048–8,192 tokens within the remaining model
capacity; a truncated attempt can grow it. It folds every
supplied transcript record into the previous summary in chronological portions;
an oversized record continues at a UTF-8 boundary in the next portion. Failed
compaction leaves the previous context in place. A learned summary output
allowance survives resume and resets when the model or effort changes.

`context/view.rs` creates a stable archived-request prefix at each compaction,
retaining the first and latest user requests within its byte budget. It also
replaces large older tool results with previews and history references.
`context/source.rs` tracks original roles, user requests and tool ownership;
`agent/history.rs` exposes paged original messages through `read`.

Each request attempt records its operation, actual effort, timing and the usage
counters the provider reported, including failed attempts with reported usage.
Reasoning is a subset of output tokens. Background titles use the same model at
low effort and participate in request accounting; a 512-token output cap applies
only on routes supporting it. The backend resolves maintenance effort before
sampling and accounting. Claude falls back to default when its catalog does
not advertise the requested level, without changing the active task's selection.

The native communication and behavior instructions live in `agent/PROMPT.md`
and are compiled into the application. Project instructions and the personal
skill catalog form the stable system prefix; permissions, temporary paths,
plans and collaboration updates form a separate final request-only state
message. Neither projection rewrites the original saved system message.
Reading a skill leaves that catalog stable;
at compaction, references to skills already read are saved separately from the
model-written summary. Skill bodies are read on demand through the agent's
`read` dispatch, alongside history references; terminal presentation
remains in the TUI. Compaction changes the active context without
rewriting original history. `Event::Context` carries the next-request estimate
to the TUI footer. See [long-running work](../usage/LONG_WORK.md) for user-visible behavior.

### Input and persistent processes

The TUI and agent share a locked inbox. Accepting a follow-up into the active turn
and removing it from the saved queue happen in the same checkpoint. Local commands
wait for the interface to become idle. Editing a pending entry holds that entry
and later messages back.

`process::start_logged` creates a command handle; `Started::supervise` owns its
timeout, cancellation, output and cleanup. Foreground and persistent commands use
that same path. Each persistent process has a monitor thread and its own stop token
in the conversation's `tools/processes.rs` registry.

`/terminal` observes this registry through a weak handle with separate output
cursors. It cannot keep a process alive. Successful conversation switches, live
session deletion and agent drop stop and join registered processes. See
[tools](../usage/TOOLS.md) for the model-facing contract.

## Questions and conversation plans

`ask::Observer` connects the terminal's input surface to the main worker without
putting rendering in the agent loop. A `user_question` journal event identifies
the original tool call. Submitting or dismissing appends a durable `user_answer`
before the waiting worker can consume it. One call carries up to four questions;
journal events written in the earlier single-question form load as one question.
The session input stores a separate answer draft per question, with its caret
and cursor row. A closed surface suspends an unanswered call and records later
calls of the turn as not executed; explicit cancellation resolves it without an
answer. Recovery finishes the same call and does not replay unrelated tool
effects. Headless input and child calls return
unavailable. Plain chat uses the same observer through its synchronous event sink.

Each successful `todo_list` set result contains the entire numbered plan. There
is no second persisted copy. `todo::latest` pairs tool results with their adjacent
assistant batch, so reused provider call IDs cannot select an unrelated result.
The request environment projects a bounded current plan from original history,
including after compaction. `/todo` caches that projection until the transcript
changes and joins delegated tasks with collaborator lifecycles. The TUI derives
from its own tool events which `set` creates a plan; only that call has a row.
Children have their own transcripts and plans.

Both tools are serial boundaries. The TUI uses the existing sheet, composer,
badge and card grammar. Ask has an independent editor, and Todo is read-only.
See [questions and plans](../usage/WORK_TOOLS.md) for the public contract.

## Collaborating agents

The main loop intercepts the `agent` control tool. It remains a serialized
boundary even in a declared parallel group. `collaboration::Observer` owns live
records, append-only receipts and worker handles; each child runs an ordinary
`Agent` on its own thread with an independent backend, tool resources, input
queue and saved conversation. Focused instructions and context replace whole
parent-conversation inheritance. Shared project-map notes and personal skills
retain the parent project's identity.

Provider requests select the catalog for the agent's role. Child
catalogs omit `agent` and `ask_user`, and agent dispatch rejects child delegation independently
of provider behavior. Existing workers keep the model, effort and routing with
which they were created.

Versioned assignment updates use the existing input queue at model boundaries.
Sent and received receipts are separate. Visible messages, tool activity and
returned reports populate inspectable records; hidden reasoning is not included.
The main agent records review explicitly against the latest returned version.
The current request environment identifies the main agent as the review owner,
includes each report's version and assessment state, and names the review and
close actions. A textual completion does not change collaborator state.

Each record keeps the collaborator's activity as one ordered log
(`collaboration/log.rs`): received assignments, visible messages, calls and
returns, referring to the bounded `updates` and the `tools` entries rather than
copying them. Records saved before the log existed are restored with their
assignment, calls, messages and outcome in that order. A changed file's call
also records its `+` / `−` line counts; its hunks stay only in the child's own
session events. `collaboration/changes.rs` reads them from the running child's
events, or from its saved journal under `child-home` once it no longer runs.

Receipts append to the parent session even while its main model is idle. The TUI
drains worker events and observer receipts with sequence deduplication, keeping
historical snapshots immutable. `tui/state/agents.rs` places each receipt as a
finished `collaboration` tool row whose result is the event, so it joins Work
chapters, expands with `Ctrl+O` and renders through `tui/cards/agents.rs`.
Placement comes from what the receipt records (`collaboration/placement.rs`),
not from when the TUI sees it: `work`, the label of the agent call whose
observer handle wrote it; `in_turn`, whether a main-agent turn was running; and
`in_call`, whether one of its calls was. A labelled receipt joins its call's
chapter; another joins the chapter it follows inside a turn; one written outside
a turn is lone. It is inserted before streaming prose, the position replay also
derives from `after_message`. Replay closes the open turn before a receipt
written outside a turn, reopens one for a continuation's later calls, and moves
a receipt written while a call ran after that call's record, where it was shown
live. Receipts saved before these fields take the label of the agent call saved
after them, and their restored unknown outcomes stay lone. Its `/agents` panel
reads current snapshots, and inside an agent `tui/agents/conversation.rs` turns
the ordered log into transcript items for the main renderer, with edit hunks
read from the child's session when the counts still match; the rows are cached
until the record, apart from its activity line, the width or the read changes
change. Opening a result does not review it. A new live return can start
`Agent::continue_collaboration` through the ordinary model loop without a
synthetic user message. Queued input wins, explicit cancellation suppresses the
continuation. Resume schedules one continuation for returned reports in still-open
assignments, including sessions saved before explicit closure was used. The main
agent makes the review and closure decisions; restoration never fabricates a
review. Closed historical receipts do not trigger inference. Plain stdin remains
line-based and does not poll idle workers, but handles unfinished review on resume.

Storage lives under the parent project bucket's `collaboration/<session-id>/`.
Child histories have a separate `child-home` so the main conversation picker
cannot open them as delegating agents. Session changes stop workers before
replacing the observer. Interrupted running work restores as unknown and is
never replayed. Successful close joins the worker, releases its tool resources,
saves a final `closed` receipt and removes the live record. Restore excludes
already closed records, including legacy records with `workspace_closed=true`.
An isolated copy is prepared before the `created` receipt that owns it. Binding
a resumed session under its exclusive lease (`collaboration/restore.rs`)
removes copies in that session's storage that no restored record names, through
the owner-verified, link-safe close path. Copies it cannot verify stay, are
reported in the resume status and keep their numbers out of new IDs.
A monotonic counter reconstructed from receipts keeps IDs unique after removal.
The active snapshot and repeated model context therefore depend on open work;
the durable chronological archive still grows with the conversation. Closing a
workspace preserves its historical record; missing or
invalid workspace ownership becomes a record diagnostic rather than breaking
the parent session.

Shared work uses current project files. Isolated work copies the project into
owned storage without Git (`collaboration/workspace/copy.rs`): one walk reads
each file once and writes the same bytes to the copy and the saved baseline
while fingerprinting them, so the baseline is what was copied. Entries the copy
cannot hold are omitted and reported rather than failing preparation; links
into the project resolve inside the copy. Copied files keep their modification
times, while comparison hashes file content regardless of those timestamps.
Comparison produces bounded evidence and a token
over baseline, child and current states; integration rechecks selected paths
and reports partial outcomes. Closing a copy made by the earlier Git worktree
version also removes its `.git/worktrees` entry when that entry names the copy.
File isolation does not restrict commands or external effects. See
[collaboration](../usage/COLLABORATION.md) for the user contract and
limitations.

## Session storage

`sessions/` stores conversations in buckets derived from the canonical project
directory. Loading also validates the directory recorded in the journal.
Exclusive writer leases prevent two agents from updating the same session;
summary caches accelerate listing without replacing journal validation.

New `SESSION_ID.jsonl` files use `jecode.session.journal`, version 3. Each synced,
newline-terminated transaction appends messages and events with changed metadata
and input. Streaming text uses UTF-8-safe prefix/suffix updates. Unchanged state
is omitted; previous messages remain intact.

An attachment draft created before model selection stores a null `model` in its
metadata and listing cache. It cannot contain an execution turn. Resume binds
that draft to the current selection before replacing the live conversation;
without a selection, it remains an unselected draft. Selected models retain
their string representation, including literal IDs such as `unconfigured`.
Older builds requiring a model string cannot open these new unselected drafts.

Conversation saves surround provider and tool work. The first streamed text is
saved immediately; later streaming updates and TUI input use a 250 ms save
interval. Periodic saves serialize disk access in a worker; explicit saves and
clean exit flush synchronously. The session journal calls these saves
checkpoints; they are unrelated to the workspace checkpoints below.
Automatic batches record call IDs in `pending.reads` until their results are
saved in source order. Explicit batches also mirror their IDs into this legacy
field, so an older reader treats interrupted modifying calls as unknown. They
record IDs in `pending.batch` and save each completed result, timing and
optional diff in `pending.completed` before announcing it to the interface.
Command checkpoint preparation and its running notes happen before workers
start; each modifying call retains its own recovery scope and receipt.
The controller joins all workers, then inserts canonical tool results, timing
and diff events in source order in one checkpoint. A crash restores saved
completions, marks started calls without a durable outcome unknown, marks
unstarted calls not executed, and never replays a call. Older readers may lose
the completed detail but conservatively mark the batch calls unknown. Earlier
journals without these fields remain readable.

| Condition | Recovery |
| --- | --- |
| Failed append | Attempt rollback to the last acknowledged journal length; require reopening if rollback fails. |
| Torn final transaction | Preserve incomplete bytes separately and recover the preceding complete checkpoint. |
| Corrupt complete transaction | Report the error and leave the file unchanged. |
| Earlier format | Report it as unsupported and leave the file unchanged. |

Session-owned output and temporary directories have ownership markers. Attachments
share a pool within the project bucket and are collected when no saved session
references them. Operational details live in [sessions](../usage/SESSIONS.md),
[temporary files](../usage/TEMPORARY.md) and [attachments](../usage/ATTACHMENTS.md).

## Workspace checkpoints

`checkpoints/` saves the before state of one modifying tool call under
`~/.jecode/checkpoints/<project-name>-<canonical-root-id>/`. Packs sit directly
in that directory; `.internal/` holds `workspace.id`, `identity.lock` and
`leases/`. The user contract is in [checkpoints](../usage/CHECKPOINTS.md). No
baseline scan or global history is kept.

| Part | Responsibility |
| --- | --- |
| `store.rs`, `format.rs`, `capture.rs` | Independent immutable packs, streamed preimages, checked metadata, atomic durable publication, readers and individual expiry. |
| `paths.rs`, `state.rs`, `files.rs` | Explicit scope discovery, complete-directory proofs, path identity/confinement, bounded observation and staged installation. |
| `restore.rs`, `restore/` | Stateless inspection tokens, current-byte checks and restore with precise per-path undo. |
| `locks.rs`, `pressure.rs` | Cancellable striped path leases and advisory free-space admission. |
| `session.rs`, `actions.rs`, `show.rs`, `command.rs`, `render.rs` | File/Bash preflight receipts, three model actions, saved reads and local commands. |

Each call writes one `.rcp` file: a small header, raw file bytes, checksummed
metadata with offsets and a footer. Publication syncs the pack before rename;
like the file tools, it leaves the directory entry unsynced. There is no shared
object graph, global replay, after-state payload, delta, watcher, group,
acceptance action or compaction.

File-tool leases span reading through installation. Bash captures declared files
and eligible directory trees before launch, then releases its capture lease;
completion and background-process monitoring do no checkpoint work. Only a
fully visited and rechecked directory scope establishes absence for new children.
Walks skip Git-ignored directories using an owned `.gitignore` reader and record
the rules with the skipped paths. Restores use those captured rules, including
for later-created directories, while explicit scopes can override implicit
exclusions. Unsupported/reparse or unreadable
directories cannot establish completeness.

Inspection hashes selected current states into a stateless expectation token.
Restore requires it or explicit `latest`, verifies source content, captures undo
and rechecks the digest of current bytes before each install. The result
describes partial progress and undo for exactly the mutated paths; a file's
replacement is staged before destructive cross-type operations. A crash leaves
the published preimage available without needing completion classification.

Creation fixes pack expiry at 24 hours. Reads and activity do not renew it. The
session's weak maintenance worker sweeps recognized workspace directories once
per minute; offline commands sweep too. Shared pack readers delay physical
removal, and stopped applications clean up on next use. Expiry removes individual
packs without copying retained data; workspace identity and lease metadata are
not subject to the 24-hour deadline.

## Native boundaries

Rust launches owned, build-embedded native adapters through
`std::process::Command`. They use OS APIs without Rust crates or unsafe Rust FFI.

| Boundary | Implementation and requirements |
| --- | --- |
| Windows processes | The build-embedded C# supervisor creates commands atomically inside Win32 Job Objects and reports completion through a private named pipe. A separate stop channel keeps cancellation independent of blocking completion reads; the supervisor acknowledges termination after the job has no active processes. Childless native HTTP and account crypto adapters use `process::run_leaf`; cancellation and timeouts kill them directly. |
| Checkpoint disk space | Build-embedded adapters query `GetDiskFreeSpaceExW` on Windows and `statvfs` on Unix. Failed queries prevent payload admission and can retry after five seconds. |
| Unix processes | An embedded C guardian owns a process group and watches a private owner pipe. It uses libc process, signal and FIFO APIs for cancellation-aware binary stdin and cleanup. |
| Windows terminal | A build-embedded C# adapter reads `CONIN$`, waits on the console input handle and sends key, wheel and geometry events through a named pipe. Requires Windows 10+ and the system .NET Framework. |
| Unix terminal | An embedded C adapter uses `termios` and `/dev/tty`, with a native guardian for terminal restoration. Requires a VT-compatible terminal. |
| Isolated collaborator workspaces | Owned Rust code copies project files, compares the snapshot with the current project and integrates selected paths. Git and `HEAD` are unnecessary; no remote, branch or commit is created. |

Process guardians stop owned descendants on cancellation, timeout or owner loss.
Unix children that create another process group or session leave that cleanup
boundary. A command interrupted without a recorded result has an unknown outcome
on resume.

The TUI uses the alternate screen, 24-bit color and changed-row VT painting.
Native adapters normalize wheel input, enable bracketed paste and restore modes,
the cursor and the previous shell screen on exit or owner loss. Simultaneous loss
of the Unix owner and guardian can prevent restoration. `--plain` bypasses the TUI
adapter; see [scrolling and resize](../tui/RESIZE.md).

Native fixture commands and host-specific checks are in [platform verification](PLATFORMS.md).

## Provider transport

`backend/` owns the inference contract: completion text and flat tool calls,
typed input/output/cached-token usage, context limits, progress updates and
failures. The agent can retain its transcript, execute tools, compact context
and save a session without importing an OpenRouter type. `conversation/` keeps
portable message fields and tags native extras with their backend, model and
verified account registration when the provider uses account sessions.
Only the matching adapter receives those extras on a later request. Legacy
untagged OpenRouter messages are assigned their archived origin on a transient
request copy; the saved originals remain intact. `auth/` represents user-scoped
credential material and account identity, while `backend::Services` exposes
catalog, credential checks and optional routing choices to setup and the TUI.
The existing user-scoped configuration stays readable. OpenAI and Anthropic
OAuth records are separate from API-key settings. `auth/accounts.rs` dispatches
account discovery and lifecycle operations without mixing provider identities.
Shared redaction sources mask newly rotated
tokens in already-created tools, output readers and persistence handles.

`backend/openai/` translates the same transcript and local tools into stateless
Responses requests using the selected ChatGPT registration. It preserves ordered
native output items and assistant phases, and requires `response.completed`
before returning executable calls. `auth/openai/` owns browser authorization,
PKCE, ID-token validation, protected records, serialized refresh and logout.
See [OpenAI](OPENAI.md) for the route contract and native crypto boundary.

`backend/anthropic/` maps canonical messages and local tools to streamed
Messages requests. It retains signed thinking and native content blocks only
for their originating account and model. `auth/anthropic/` owns Claude browser
authorization, profile identity, protected records, serialized refresh and local
sign-out. It uses the same owned native HTTP and cryptography boundary, without
a Claude Code runtime or SDK. The subscription route uses a compatibility profile; see
[Anthropic](ANTHROPIC.md) for the compatibility profile and verification limits.

`backend/openrouter/` uses the fixed HTTPS OpenRouter API through the native
HTTP adapter with TLS certificate and hostname validation. The JSON body and
selected key travel on child stdin; arguments and files contain neither. Test
builds can use loopback endpoints.
The request has no harness-imposed byte limit. Model context and service limits
remain separate. Process input delivery errors retain the child's exit status
and captured diagnostics, so a closed pipe cannot hide the native failure reason.
Native helpers and tool subprocesses remove credential environment variables
registered by the available backends before launch.

The adapter wraps flat tool calls and the neutral tool catalog in OpenRouter's
function-call wire shape. It retains ordered reasoning fields and any native
tool-call extensions only for a matching backend and model. Attachment references
are materialized at this boundary; PDFs use native model input when supported
and OpenRouter's free Cloudflare parser otherwise. The adapter applies the model,
effort and endpoint routing to ordinary and summary requests. Summary requests ask for
plain text without `response_format`; Jecode sets no explicit prompt-cache
markers. Provider diagnostics retain the underlying code, parameter and message
after credential masking and length bounding. See
[configuration](../usage/CONFIGURATION.md) for routing and overrides.

`agent/backend_tests.rs` exercises two synthetic backends with different wire
layouts through tool execution, compaction, save, resume and a failed switch.
All three adapters have loopback fixtures for their HTTP and
streaming paths.

Auto routing keeps OpenRouter fallbacks. With enough fresh comparable endpoint
statistics, it requests throughput near or above half the median and first-token
latency near or below twice the median. These are soft preferences. A specific
endpoint disables fallbacks and requires a compatible listing within its freshness
window before the turn starts.
