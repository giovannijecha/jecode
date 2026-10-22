# Architecture

Jecode is one Cargo package built from the Rust standard library and original
project code. The agent owns conversation state and tool execution; CLI and TUI
consume its events. Terminal rendering stays outside the agent loop.

## Modules

Paths are relative to `src/`.

| Area | Modules |
| --- | --- |
| Entry and setup | `main.rs`, `cli.rs`, `bootstrap.rs`, `setup/` |
| Settings and model effort | `config/`, `effort.rs` |
| Agent and context | `agent.rs`, `agent/`, `context.rs`, `context/`, `events.rs` |
| Input during a turn | `inbox.rs`, `agent/input.rs` |
| Personal skills | `skills.rs`, `skills/`, `agent/skills.rs` |
| Tools and output | `tools/`, `file_change.rs`, `process.rs`, `process/`, `output.rs`, `output/` |
| Saved conversations | `sessions/`, `agent/persistence.rs` |
| Workspace checkpoints | `checkpoints.rs`, `checkpoints/`, `agent/checkpoints.rs`, `session/checkpoints.rs` |
| Temporary working files | `scratch/`, `agent/temporary.rs` |
| Attachments | `attachments/`, `tools/attachment.rs` |
| OpenRouter | `openrouter/` |
| Interfaces | `session.rs`, `session/`, `console.rs`, `tui/` |
| Shared utilities | `json.rs`, `cancel.rs`, `redact.rs`, `copy.rs`, `clipboard.rs`, `export.rs` |

## Request lifecycle

1. `bootstrap.rs` resolves configuration, working directory and execution mode.
   Interactive stdin/stdout selects the TUI; prompts, pipes and `--plain` use the CLI.
2. The agent snapshots `JECODE.md` files in the launch directory and its real
   subdirectories before recording the request. Each file carries its directory
   scope; the combined snapshot applies throughout the turn, including after
   compaction. Parent directories are never searched.
3. The context layer projects the active conversation for OpenRouter. The client
   streams text, reasoning fields and tool calls through agent events.
4. The agent saves tool boundaries, executes calls and records their results.
   Follow-up messages from the shared inbox join before the next model request.
5. Completion, cancellation or failure checkpoints the conversation. The interface
   renders the result and keeps unsent input available.

The harness supplies tools, context and history. It adds no completion gates,
repeated-command refusals or model-output validation.

File writes and exact edits capture contextual changes at execution time.
`file_change.rs` owns their text alignment and serializable hunks, independently
of terminal rendering. A local `tool_change` event is saved with the tool-result
checkpoint and replayed to the TUI; it is excluded from provider messages and
context estimates. The model-facing tool result remains unchanged.
Optional capture bounds input size and alignment work; unavailable diffs never
reject an otherwise valid write or edit. Local `turn_start` events preserve
execution-turn grouping across queued follow-ups and resume.

### Context and compaction

`context.rs` derives the input budget from the Auto-compact percentage and the
effective model window, lowered when a provider rejection reveals a smaller
ceiling. The trigger, result previews, retained requests and summary sizing use
that budget. Prompt estimates calibrate from provider usage when available.

`agent/summary.rs` asks the same model for a plain-text handoff containing the
previous summary, original user requests and the newest transcript records that
fit. Older omitted records remain readable. A learned summary output allowance
survives resume and resets when the model or effort changes.

`context/view.rs` preserves recent user requests verbatim within its byte budget
and replaces large older tool results with previews and history references.
`context/source.rs` tracks original roles, user requests and tool ownership;
`agent/history.rs` exposes paged original messages through `read`.

The native communication and behavior instructions live in `agent/PROMPT.md`
and are compiled into the application. The current environment, project
instructions and personal skill catalog are projected separately from
the original saved system message. Skill bodies are read on demand through the
agent's `read` dispatch, alongside history references; terminal presentation
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

## Session storage

`sessions/` stores conversations in buckets derived from the canonical project
directory. Loading also validates the directory recorded in the journal.
Exclusive writer leases prevent two agents from updating the same session;
summary caches accelerate listing without replacing journal validation.

New `SESSION_ID.jsonl` files use `jecode.session.journal`, version 3. Each synced,
newline-terminated transaction appends messages and events with changed metadata
and input. Streaming text uses UTF-8-safe prefix/suffix updates. Unchanged state
is omitted; previous messages remain intact.

Conversation saves surround provider and tool work. The first streamed text is
saved immediately; later streaming updates and TUI input use a 250 ms save
interval. Periodic saves serialize disk access in a worker; explicit saves and
clean exit flush synchronously. The session journal calls these saves
checkpoints; they are unrelated to the workspace checkpoints below.

| Condition | Recovery |
| --- | --- |
| Failed append | Attempt rollback to the last acknowledged journal length; require reopening if rollback fails. |
| Torn final transaction | Preserve incomplete bytes separately and recover the preceding complete checkpoint. |
| Corrupt complete transaction | Report the error and leave the file unchanged. |
| Version 2 journal | Read existing records and append version 3 continuations. |
| Version 1 snapshot | Migrate on explicit resume, retaining the original snapshot and backup. |

Session-owned output and temporary directories have ownership markers. Attachments
share a pool within the project bucket and are collected when no saved session
references them. Operational details live in [sessions](../usage/SESSIONS.md),
[temporary files](../usage/TEMPORARY.md) and [attachments](../usage/ATTACHMENTS.md).

## Workspace checkpoints

`checkpoints/` keeps file history for each canonical workspace root under
`~/.jecode/checkpoints/<id>/`, independently of conversations and of the
terminal. User-facing behavior is in [checkpoints](../usage/CHECKPOINTS.md).

| Part | Responsibility |
| --- | --- |
| `store.rs`, `journal.rs`, `frame.rs`, `record.rs`, `index.rs` | One archive generation: checksummed frames in 64 MiB append-only segments, objects stored whole or as bounded prefix/suffix deltas, and the in-memory index replayed from them. |
| `locks.rs`, `activity.rs`, `identity.rs` | Operating-system file locks for archive writes and striped path leases; live-session descriptions; one archive per canonical root. |
| `workspace.rs`, `inventory.rs`, `scan.rs`, `capture.rs`, `policy.rs` | Bounded background discovery, selective protection of large scopes, reconciliation and parallel capture with bounded live-content buffers. |
| `quota.rs` | Admission under the shared store lock: one traversal counts total and active-generation logical bytes, with a hard 1 GiB archive ceiling and a 512 MiB active generation ceiling. |
| `watch.rs`, `watcher.cs` | Windows change notifications through an owned .NET helper. |
| `mutation.rs`, `reconcile.rs`, `points.rs` | Tool file changes and commands, resolution of operations left prepared by exited processes, and points. |
| `restore.rs`, `history.rs`, `gc.rs` | Restores with undo and conflict checks; pins, limits and generation compaction. |
| `actions.rs`, `command.rs`, `render.rs`, `session.rs`, `show.rs` | The model tool, `jecode checkpoints` and `/checkpoints` over shared actions; notices; historical reads. |

Every archive write runs under the archive lock and first replays frames that
other processes appended. A tool's file change durably records a prepare frame
with the previous and intended state before the file changes, and a completion
afterward. A successful write completes from the tool's bytes and the file's
metadata; the path is then reread off the call's path, or before the next
point, to observe concurrent changes. Windows measurements show a delay when
reopening newly written large files; its cause has not been isolated.
Recovery resolves an interrupted operation by observing the file.
Parallel observation preserves path order and hands off bounded content batches,
including during restores. Size hints from discovery cannot bound live data:
another writer may have grown the files. A failed later batch can leave truthful
earlier observations in history, while no restore installation has begun.
Restores durably plan per-path progress, save the current state, install
targets and record a point with undo. Compaction copies retained history into a
new generation, publishes it through `CURRENT` and leaves the old one to its
readers until they move on. Idle workers follow these moves and remove released
generations. The latest named point remains a state anchor even when the
retention horizon advances through versions from a long turn. A new point or
an unpin can retry a blocked collection without waiting for more byte growth.

The agent records a point at each turn, wraps each tool call and delivers
notices about history changed outside its own calls before the next request.
Failures make a call unprotected and never block it. `tools/` returns each
protection status, the TUI renders it, and the offline CLI opens the archive
without a background worker or provider.

## Native boundaries

Rust launches installed executables through `std::process::Command`. Owned Bash,
PowerShell and C# helpers provide native functionality without Rust crates or
unsafe Rust FFI.

| Boundary | Implementation and requirements |
| --- | --- |
| Windows processes | PowerShell compiles the owned C# supervisor once per source version into `~/.jecode/cache/process/`. It creates commands atomically inside Win32 Job Objects and reports completion through a named pipe. The original PowerShell path is the fallback. |
| Windows file notifications | An owned C# helper reports paths through `FileSystemWatcher` over stdin/stdout. Its marker echo checks helper responsiveness, not filesystem ordering. Ordinary points drain delivered observations; Bash preparation and completion use a bounded settling window. Queue overflow requests reconciliation. Without the helper, checkpoints scan. |
| Unix processes | A Bash guardian owns a process group and watches a private owner pipe. `kill` handles timeout/cleanup; `mkfifo`, `rm` and `rmdir` support cancellation-aware binary stdin through a private FIFO. |
| Windows terminal | An owned PowerShell/C# adapter reads `CONIN$`, waits on the console input handle and sends key, wheel and geometry events through a named pipe. Requires Windows 10+ and `Add-Type`. |
| Unix terminal | Bash and `stty` save terminal modes, read `/dev/tty` and restore the terminal through a guardian. Requires a VT-compatible terminal. |

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

`openrouter/` uses the fixed HTTPS OpenRouter API. Requests and credentials reach
curl through stdin with curl's default configuration disabled and TLS certificate
validation enabled. Test builds can use loopback endpoints.

The client preserves assistant content, tool calls and ordered reasoning fields
for compatible follow-up requests. It applies the selected model, effort and
endpoint routing to ordinary and summary requests. Provider diagnostics retain
the underlying code, parameter and message after credential masking and length
bounding. See [configuration](../usage/CONFIGURATION.md) for routing and overrides.

Auto routing keeps OpenRouter fallbacks. With enough fresh comparable endpoint
statistics, it requests throughput near or above half the median and first-token
latency near or below twice the median. These are soft preferences. A specific
endpoint disables fallbacks and requires a compatible listing within its freshness
window before the turn starts.
