# Long-running work

Jecode imposes no model-request count, tool-call count or elapsed-time ceiling on
a turn. It continues while the selected model requests tools. A final response,
user interruption, or an error that needs intervention ends the turn. It does not
guess that repeated commands or quiet processes are stuck, second-guess a final
response, or alter the model's reasoning strategy.

## Connection recovery

Temporary network failures, incomplete SSE connections, rate limits and temporary
provider errors retry the current request until recovery or interruption. The
delay increases from 2 seconds to 64 seconds and respects a longer `Retry-After`,
including an HTTP date. This delay ceiling controls request frequency; it does
not limit attempts or turn duration. In the TUI on Windows, Linux and macOS, Esc/Ctrl+C stop waits
through the existing cancellation mechanism. Credentials, exhausted credits, invalid requests and
other permanent errors return control to the user.

A completed response containing neither text nor executable tool calls, or a
provider error explicitly reporting an empty response, is retried up to three
empty responses for that request. Repeated emptiness returns an error
with the original context preserved, instead of leaving the job in an endless
retry loop. This bound does not apply to ordinary temporary network errors.

Curl has a 10-second connection deadline. The process runner detects 120 seconds
without any response bytes; every received chunk, including SSE comments, resets
that idle timer. These conditions start recovery; they do not terminate the
overall job. The process runner has no overall deadline for API requests.

An incomplete response never executes its tool calls. Validated tool results
already in the conversation remain there while the following model request is
retried. Failed attempt text and diagnostics are retained as local evidence in
the session and export, separate from completed provider messages. The TUI shows
reconnection status above the composer; input and interruption remain available.

## Active context and original history

The original provider messages and tool results remain in the saved session,
display source and JSON export. The request sent to the model is a separate view:
the original system instructions, the most recent original user requests, the
conversation summary when present, and the messages since the last compaction.
After compaction, recent user requests stay verbatim, newest first, within about
a quarter of the input budget; older ones are listed as omitted and remain
available through `history:requests`.
After a model change, older provider-specific reasoning fields are removed only
from the request view.

OpenRouter's exact model catalog entry supplies `context_length` and, when
available, `max_completion_tokens`. The latest response's `prompt_tokens` anchors
the input estimate; new serialized message bytes provide a conservative allowance
until another measurement arrives. After a projection changes through compaction,
Jecode can use the last successful request's measured token density with a
threefold margin, capped at the original serialized-byte estimate. Newly appended
messages remain byte-counted until measured. Without calibration, serialized
UTF-8 bytes of messages and tool definitions remain the conservative fallback.
Calibration survives resume, resets on a model change and is discarded after a
provider context rejection or a response without valid prompt-token usage. An
unmeasured response cannot pair an older token count with a newer request size.
It is an estimate rather than an exact tokenizer;
provider rejection still invokes context recovery. The reply reserve derives from the selected model's
window and observed completion size. It is not a fixed token ceiling for jobs.

Ordinary tool turns and continuity requests keep the selected reasoning effort.
When the exact model catalog declares an output maximum, Jecode requests that
maximum explicitly, reduced only by space needed for the input in the available
context. Reasoning and visible output share that allowance; Jecode does not
silently lower effort or impose a separate fixed reasoning budget. Without a
declared output maximum, ordinary turns use the provider default and continuity
requests retain their adaptive context-based allowance. A truncated ordinary
response reports the model, effort and requested output allowance, preserving
completed tool results without executing incomplete calls.

Before the view fills the input budget, Jecode first turns large older tool
outputs into explicit context previews with `history_reference`; new tool
results remain complete until another compaction, and the original stored
results are never replaced by previews. When that is not enough, Jecode
summarizes everything since the previous compaction with the **same selected
model**, with tools disabled. The live view then restarts from the summary.

The summary is plain text written by the model: a handoff with the goals,
constraints, decisions, completed work, open errors and next steps that the
continuation needs. Each summary request carries the original user requests and
the previous summary, so repeated compaction folds new work into one updated
handoff. Jecode masks known credentials but does not validate or rewrite it; the original transcript, saved
output and `history:N` references remain available for anything the summary
omits. The next request carries the summary as a user message that tells the
model another model started the task, its tool work is still in place, and
`history:N` holds the originals.

Compaction makes one summary request. Source-labelled transcript records, with
previews for large outputs, are included newest first; when they do not all fit,
the oldest are left out with a note pointing to `history:N`. A provider context
rejection halves the transcript share and retries; a truncated summary doubles
the output allowance, always leaving a quarter of the window for transcript
input. A provider context rejection can also reduce the learned capacity and
trigger compaction even when catalog metadata is unavailable.

The summary must reduce the request view and be saved before work continues. An
empty, interrupted or unsaved summary leaves the previous view available and
retains the source history. A provider output limit on an ordinary response still
reports an incomplete response rather than executing partial calls. The retained
session can be continued explicitly.

The TUI reports compaction and completion in the conversation. Compaction and
retries are part of the original turn: its activity timer does not restart, its
draft/queue are not submitted, and its tools are not replayed. Resume restores
the saved context view and original transcript without starting work.

`read(path="history:N")` retrieves an original message by its zero-based index.
Tool results are returned as structured text and provider reasoning fields are
excluded. `history:requests` retrieves the complete ordered user requests, and
`history:memory` retrieves the current summary.
All support the same line and UTF-8 byte pagination as file reads, mask known
credentials and remain read-only after compaction and explicit resume.
Tool-result reads include `request_history_reference` to retrieve their original
arguments, including exact content from a previous write.

## Foreground commands and output

Bash runs ordinary `bash -c` commands: Jecode does not refuse repeated commands,
force `errexit` or `pipefail`, or track file changes between calls. The model
reads the exit code and output and decides what to do next. See
[TOOLS.md](TOOLS.md) for fields and usage.

Bash waits for the command without an imposed timeout, even if it stays silent.
The model may request a positive `timeout_seconds` for a particular command;
there is no 600-second maximum. Input remains available in the TUI and cancellation
terminates the command's Windows Job Object or Unix process group. Ctrl+Q performs
that cleanup and exits; Ctrl+C never exits the TUI. The Windows
supervisor waits for all Job members, including children surviving their original
shell, and closes the Job on timeout, cancellation or loss of the Jecode process.
It assigns the target to the Job during process creation. On Unix, an owned
Bash guardian starts before the target and watches a private owner pipe;
closing it, including by abruptly killing Jecode, stops the command group.
Children that leave the group are outside this cleanup boundary. Line-based
chat uses the host's Ctrl+C exit behavior, with the same ownership cleanup.
Resume retains an unknown outcome instead of repeating it. Jecode has no
background worker service.

If saving output or confirming process cleanup fails after a command has started,
its result reports an unknown
outcome and retains output references. The command may already have changed files;
the agent receives that evidence before deciding what to inspect next.

Each command saves stdout and stderr to owned files in the current folder's
user-scoped session bucket. Known configured keys are masked across stream chunk
boundaries before disk writes. The result includes the last 64 KiB of each stream,
byte counts and read-only `output:SESSION_ID:OUTPUT_ID:stdout` / `:stderr` references. `read`
accepts those references after the command and after explicit resume. They do not
allow access to other folders' logs or the configuration file. `write` and `edit`
cannot modify output references. Complete output is not embedded in every future
request; the model can retrieve the portions it needs.
Marked per-session output directories let session deletion remove those streams and
unreferenced crash remnants. Older flat output references remain readable and
are kept during deletion because their ownership cannot be established.

`read` pages contain at most 2,000 lines and 64 KiB. `next_offset` continues at a
line; `next_byte_offset` continues a split long line without losing characters.
Byte mode seeks directly to the requested position and returns unnumbered text
with its byte range, so subsequent pages do not rescan preceding output.
These are bounded pages, not file-size or job limits. UTF-8 text files have no
fixed 1 MiB ceiling; writes and edits still check paths, exact matches, text and
read-only permissions before replacing files.

## Persistence and practical boundaries

One-off scripts and artifacts can use the folder/session temporary area. It
survives completed turns, interruption, exit and explicit resume. Tool contracts
and model requests retain access instructions through context compaction. Cleanup
is explicit through `/tmp clean`, with intent saved before deletion; complete
command streams and conversation history are stored separately and retained.
Ctrl+D and Enter in `/resume` remove a selected conversation and its owned files.
See [TEMPORARY.md](TEMPORARY.md).

Sessions append synced transactions rather than rewriting complete history for
each model/tool/input checkpoint. Streaming checkpoints save UTF-8-safe text
increments and omit unchanged metadata, input and compaction summaries. Session
listing uses rebuildable lightweight caches; explicit resume validates the journal
and reconstructs message/event order in one traversal. Validation checks newly
appended messages and events. There is no fixed session-size ceiling. Legacy snapshots remain readable;
torn journal tails can recover the preceding complete checkpoint. Details are in
[SESSIONS.md](SESSIONS.md).

Work runs while Jecode is open. Closing or losing the process requires explicit
folder-scoped resume. An interrupted tool outcome can be unknown; recovery never
blindly executes it again. This is not a daemon, detached scheduler or guarantee
that the selected model will complete an arbitrarily long objective. Runtime
memory still holds the original conversation/display source. RAM, disk space,
provider capacities, availability and account credit remain real constraints.

Resumed model requests use the current Jecode system instructions, including in
context projections and their capacity estimates. The original saved system
message remains unchanged in history for inspection and export.

## Verification

Loopback tests use synthetic credentials and cover an 80-request turn,
56 consecutive temporary errors followed by recovery without repeating a command,
server-directed delay and cancellation, incomplete streams, credential failure,
repeated compaction and resume, provider context rejection, failed summary saves,
large tool-output previews, history retrieval, ordered user corrections, empty
responses, complete output retrieval, long UTF-8 lines, sessions over 64 MiB, legacy migration
and crash recovery. Process tests cover cancellation and timeout after the shell
has exited; Windows and Linux tests also cover owner disappearance. Streaming
journal tests cover bounded growth, UTF-8 replacement and torn checkpoints. These
exercise the mechanisms; they are not a multi-day live OpenRouter soak test.
Windows and Linux coverage includes the recovery and persistence checks.
Linux checks execute under WSL with native tmpfs fixtures; macOS has all-target compilation coverage for
Intel and Apple Silicon, with native runtime checks still required.

The isolated native Windows console probe verifies visible recovery status,
compaction, a queued local command, a retained draft, width/height changes and
JSON export together. It uses loopback HTTP and synthetic credentials. Run from
the repository root in an interactive Windows terminal:

```powershell
cargo test --locked --offline tui::long_work_smoke::long_work_windows_smoke -- --ignored --exact --nocapture --test-threads=1
```
