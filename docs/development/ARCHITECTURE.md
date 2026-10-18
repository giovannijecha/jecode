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
| Tools and output | `tools/`, `process.rs`, `process/`, `output.rs`, `output/` |
| Saved conversations | `sessions/`, `agent/persistence.rs` |
| Temporary working files | `scratch/`, `agent/temporary.rs` |
| Attachments | `attachments/`, `tools/attachment.rs` |
| OpenRouter | `openrouter/` |
| Interfaces | `session.rs`, `session/`, `console.rs`, `tui/` |
| Shared utilities | `json.rs`, `cancel.rs`, `redact.rs`, `copy.rs`, `clipboard.rs`, `export.rs` |

## Request lifecycle

1. `bootstrap.rs` resolves configuration, working directory and execution mode.
   Interactive stdin/stdout selects the TUI; prompts, pipes and `--plain` use the CLI.
2. The agent records the request and snapshots the launch directory's `JECODE.md`.
   That snapshot applies throughout the turn, including after compaction.
3. The context layer projects the active conversation for OpenRouter. The client
   streams text, reasoning fields and tool calls through agent events.
4. The agent saves tool boundaries, executes calls and records their results.
   Follow-up messages from the shared inbox join before the next model request.
5. Completion, cancellation or failure checkpoints the conversation. The interface
   renders the result and keeps unsent input available.

The harness supplies tools, context and history. It adds no completion gates,
repeated-command refusals or model-output validation.

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

The current environment and project instructions are projected separately from
the original saved system message. Compaction changes the active context without
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

Checkpoints surround provider and tool work. The first streamed text is saved
immediately; later streaming updates and TUI input use a 250 ms save interval.
Periodic saves serialize disk access in a worker; explicit checkpoints and clean
exit flush synchronously.

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

## Native boundaries

Rust launches installed executables through `std::process::Command`. Owned Bash,
PowerShell and C# helpers provide native functionality without Rust crates or
unsafe Rust FFI.

| Boundary | Implementation and requirements |
| --- | --- |
| Windows processes | PowerShell compiles the owned C# supervisor once per source version into `~/.jecode/cache/process/`. It creates commands atomically inside Win32 Job Objects and reports completion through a named pipe. The original PowerShell path is the fallback. |
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
