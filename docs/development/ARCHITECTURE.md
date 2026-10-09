# Architecture

Jecode uses one Cargo package, the Rust standard library and original owned code.
The agent loop stays independent of terminal rendering; CLI and TUI consume its
events through their own interfaces.

## Modules

Paths below are relative to `src/`.

Modules stay around 500 lines or fewer and split by responsibility:

- `main.rs`, `cli.rs`, `bootstrap.rs`: dispatch, arguments and startup.
- `config/`, `effort.rs`: user paths, settings, reasoning effort and atomic storage.
- `setup/`: guided key input and model selection.
- `session/`, `session.rs`, `input.rs`, `console.rs`: CLI interaction and rendering.
- `sessions/`, `agent/persistence.rs`: folder-scoped storage, checkpoints and recovery.
- `scratch/`, `agent/temporary.rs`: session working files, ownership and explicit cleanup.
- `tui/`: Windows and Unix terminal adapters, shared multiline editor, prompt history, message queue,
  selectors, conversation state, tool presentation, semantic theme, styled text,
  Markdown/code highlighting, cached transcript layout, resize scheduling,
  anchored scrolling, source reflow, commands and background
  workers. Each responsibility has its own module.
- `agent.rs`, `agent/`, `context.rs`, `context/`, `events.rs`: conversation/tool loop,
  recovery, context projection, validated continuity memory, execution facts and
  compaction independent of rendering. `agent/history.rs` exposes read-only
  paginated original messages and the full retained operational ledger through
  the existing read tool. `context/memory/projection.rs` bounds the active
  completed-work view without archiving constraints or unfinished work.
  `context/memory/proof.rs` validates completed and resolved entries against
  original user/tool evidence and identifies rejected fields and references.
  `context/memory/request.rs` requires explicit review of prior requirements
  on a newer user request and stamps an owned review boundary. Subsequent
  portions of that request keep delta retention. New proven resolutions remove
  exact repeated pending copies without waiving other work or file constraints.
  Preview progress cannot make a user decision newer than that boundary stale;
  tool-proof freshness uses the accepted summary boundary rather than the
  preview cursor. Older tool results before that accepted boundary stay stale.
  Requests retain the current user's original history boundary across compaction
  and resume, so summarized actions remain attributable to their request.
  The archived request view identifies the current request from the complete
  transcript, including a newer request in the live tail. Its introduction exposes
  factual history locations rather than assigning precedence to an archived task.
  Continuity memory exposes a response schema. `context/memory/pending.rs` accepts
  pending text or a description with an optional operation kind and canonicalizes
  only that representation before native retention and proof validation. Pending
  annotations supply no completion or decision proof; unsupported fields fail.
  `context/memory/evidence.rs` derives source roles, originating user requests,
  matching tool calls and eligible proof kinds from original history. Compaction,
  execution-state views and paginated history use the same provenance. Tool
  ownership follows the original call even when a later user request intervenes.
  `context/state.rs` presents native boundaries separately from proposed memory;
  it does not instruct the model to follow an old next action.
  `agent/environment.rs` supplies the agent identity, working-directory/access
  facts and the latest original user-request reference. During completion review
  it exposes the provisional response reference, final-delivery state and live
  native blocker. These replace the general workflow and completion-review
  paragraphs; native execution guards and tool contracts remain independent.
  Session temporary paths and explicit cleanup receipts are factual views too.
  Current projections leave the saved original system message unchanged.
  `agent/project_instructions.rs` loads the launch-directory `JECODE.md` before
  each user request and keeps one snapshot for the turn. The current system
  projection includes its rules independently of compaction and saved history;
  changed rules invalidate direct prompt measurements while keeping conservative
  token calibration and the learned context ceiling.
  `context/memory/feedback.rs` exposes rejected proposals, cited source types
  and independent field/entry failures through the existing native validators.
  `context/memory/repair.rs` supplies the same native source eligibility plus
  original requests, call arguments and result facts for a structured rejected
  proposal. Repairs keep the original transcript portion boundary instead of
  resummarizing its code/output payloads and leaving a newly displaced tail.
  Missing update facts, oversized requests and provider context rejections use
  the original bounded transcript path; original history is retained throughout.
  The summary-only projection exposes earlier completed proofs as a read-only
  `completed_archive` with exact kinds and references. The response's `completed`
  field proposes additions to the native ledger, whose full prior identities
  remain retained and validated independently of the displayed archive.
  Accepted state and retry limits remain unchanged. Explicit stable targets in
  completed/resolved descriptions can appear within prose; unknown, extended or
  ambiguous targets never select a previous item by semantic guesswork.
  Summary truncation also learns a native output allowance with accepted
  context. It survives resume, resets on a model/effort change and stays bounded
  by the current context and provider output limit; live prompt measurements
  remain separate from this resource state.
  `agent/delivery.rs` records successful final-response delivery after completion
  guards and supplies that receipt to later summaries. It proves delivery only,
  uses the existing saved event format and stays outside the rendered transcript.
  `agent/repetition.rs` checks archived native Bash receipts before an exact
  command is repeated; deliberate re-execution needs an explicit reason.
- `export.rs`, `redact.rs`, `cancel.rs`: conversation snapshots, key masking and
  cooperative cancellation shared with the native process runner.
- `openrouter/`: provider API, completion parsing, incremental SSE streaming, model
  capabilities and HTTPS transport. Ordinary turns and continuity requests use
  the selected effort and declared model output maximum, bounded by remaining
  context space. Provider failures retain the underlying code, parameter and
  message after credential redaction and diagnostic length bounding.
- `json.rs`, `process.rs`, `process/`, `output.rs`, `tools/`: owned JSON, cancellable foreground
  processes, complete output storage and the five tools. `tools/watch.rs` and
  `tools/watch/fingerprint.rs` compare explicitly observed project files through
  standard filesystem APIs; native version receipts rebuild observation on resume.
  `context/evidence.rs` keeps indirect changes and check freshness independent of
  model memory, with bounded request details and separate completion notices.
  `context/evidence/comparisons.rs` reconciles an earlier incomplete comparison
  only when a later native protection status proves the exact path preserved
  against a registration predating that uncertainty. Unknown baselines, known
  changes and recorded check outcomes retain their independent evidence state.
  `tools/protection.rs` owns explicit file constraints, exact binary snapshots,
  guarded restoration and registration recovery; `tools/protection/scope.rs`
  requires review of carried registrations before a new request's mutations and
  supplies bounded related-file inventory. `protection/directories.rs` retains
  directory declarations across requests and native receipt recovery;
  `protection/decisions.rs` validates classification and exclusions, while
  `protection/registration.rs` captures existing baselines. `context/protection.rs` retains
  their native states across compaction; the agent refreshes them before accepting
  completion. Snapshots and synced intents use the session-owned output store.
  `tools/schema.rs` advertises meaningful numeric bounds, leaving read offsets
  without an artificial maximum; native argument parsing still validates them.

## Native boundaries

The native boundary is installed executables launched via `std::process::Command`.

### Unix processes

Unix commands start in their own process group. An owned Bash guardian watches
a private owner pipe before the target runs; no target inherits that control
pipe. The guardian keeps the group ID reserved and uses Bash's builtin kill on
owner loss. Normal completion drains captured output, then ends ownership and
stops any remaining members, including background children with redirected
output. Children that create another group/session remain outside this boundary.
For command stdin, `mkfifo` creates a 0600 pipe inside a 0700 system temporary
directory. Rust opens it without blocking, unlinks it once the target has opened
the reader, and sends binary input with cancellation-aware writes. Input bytes
are not saved as a temporary data file. Bash startup files and inherited shell
tracing options are disabled for the guardian. This boundary requires Bash,
mkfifo, rm/rmdir and native kill; it adds no Rust crate or unsafe FFI.
See [POSIX FIFO open behavior](https://pubs.opengroup.org/onlinepubs/007904875/functions/open.html)
and [Apple open(2)](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/open.2.html).

### Unix terminal

Bash and `stty` provide Unix terminal mode control. An owned Bash guardian saves
the original modes and restores them when its private control pipe closes,
including when Jecode disappears. Rust reads `/dev/tty` with a 100 ms terminal
read timeout, parses UTF-8 and bounded VT key sequences, and polls `stty size`
every 200 ms. Signals and XON/XOFF are disabled while editing, so Ctrl+C/Ctrl+Q
reach the application. Bracketed paste inserts multiline text without submitting
or invoking shortcuts. The guardian enables fullscreen, paste and wheel reports
before startup input is read. It disables those reports, leaves the alternate
screen, shows the cursor and ends synchronized output when its owner disappears.
No Bash startup file is loaded and no cursor-position query is needed. Normal
exit restores terminal modes and the previous shell screen.
If the guardian fails, input shutdown also releases a reader whose terminal has
returned to canonical mode. A simultaneous forced loss of the owner and guardian
or disappearance of the terminal cannot guarantee restoration.
Bash also supplies hidden API-key input.
See [POSIX terminal input](https://pubs.opengroup.org/onlinepubs/9699919799/basedefs/V1_chap11.html),
[GNU stty](https://www.gnu.org/software/coreutils/manual/html_node/stty-invocation.html)
and [xterm control sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html).

### Windows processes

On Windows, an owned PowerShell/C#
supervisor uses Win32 Job Objects, assigns the target atomically at process
creation and exchanges completion status through a local named pipe. Its Job
handle stays outside the target tree. This boundary requires Windows 10 or newer
and applies to both TUI and plain execution. Windows PowerShell compiles the
owned supervisor once per source version into `~/.jecode/cache/process/`;
later commands launch that executable directly. Compilation is bounded and
cancellable, and the original PowerShell path remains a fallback. Measured
startup costs are recorded in [PERFORMANCE.md](PERFORMANCE.md). See Microsoft's
[Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)
and [process creation attributes](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute).

### Windows terminal

The Windows TUI embeds an owned PowerShell
script and C# adapter using Win32 console APIs. It opens `CONIN$` for keyboard
and wheel records, samples geometry with `GetConsoleScreenBufferInfo`,
and sends complete geometry events through a local named pipe.
Shutdown uses a separate stdin pipe so it cannot wait behind a blocked event read.
Input/output modes and the output code page are saved and restored. The TUI uses
UTF-8 output temporarily. PowerShell and its `Add-Type` capability
must be available; `--plain` avoids the console adapter. Rendering uses terminal VT
sequences in the alternate screen buffer and requires 24-bit RGB color support.
Jecode owns conversation scrolling and repaints changed visible rows by absolute
position. Native wheel records on Windows and SGR wheel reports on Unix are
normalized into row scrolling. The native guardian enters and leaves fullscreen,
including when the owner's private pipe closes unexpectedly. Width changes
reflow retained source in bounded passes without erasing shell scrollback.
Per-write synchronized output reduces intermediate
painting on supporting terminal hosts.
The composer paints its own steady block caret; the native cursor is hidden
during the TUI and shown again when leaving it.
See Microsoft's [console input API](https://learn.microsoft.com/en-us/windows/console/readconsoleinput)
and [VT sequence reference](https://learn.microsoft.com/en-us/windows/console/console-virtual-terminal-sequences).

## Provider transport

Credentials and request bodies reach curl through stdin, with default curl
configuration disabled and TLS certificate validation enabled. Production requests use the fixed HTTPS OpenRouter API;
loopback endpoints are available only in tests.

Protocol references: [chat completions](https://openrouter.ai/docs/api/api-reference/chat/send-chat-completion-request),
[tool calling](https://openrouter.ai/docs/guides/features/tool-calling),
[streaming](https://openrouter.ai/docs/api_reference/streaming),
[reasoning tokens](https://openrouter.ai/docs/guides/best-practices/reasoning-tokens),
[model catalog](https://openrouter.ai/docs/api/api-reference/models/list-all-models-and-their-properties)
and [key validation](https://openrouter.ai/docs/api/api-reference/api-keys/get-current-api-key).
Assistant content, tool calls and ordered reasoning fields are retained for
follow-up tool interactions with the same model.
