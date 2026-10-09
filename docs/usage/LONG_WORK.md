# Long-running work

Jecode imposes no model-request count, tool-call count or elapsed-time ceiling on
a turn. It continues while the selected model requests tools. A final response,
user interruption, or an error that needs intervention ends the turn. It does not
guess that repeated commands or quiet processes are stuck, or alter the model's
reasoning strategy. Recorded file/check evidence can trigger one completion review
before a proposed final response is accepted, as described below.

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
the original system instructions, validated continuity memory when present,
anchored original user requests, and recent complete message groups. The first
objective and latest corrections remain represented even when there are many
requests. A group never separates a tool call from its results.
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

Before the view fills the input budget, Jecode compacts older groups with the
**same selected model**, with tools disabled. Recent groups use about a third of
the input budget; the newest complete group is retained when space permits.
Large older tool outputs become explicit context previews with `history_reference`.
New tool results remain complete until another compaction. The original stored
results are never replaced by previews.

Continuity memory is a JSON object with `objective`, `constraints`, `completed`,
`remaining`, and `next_action`. Completed entries name their kind and cite
original `history:N` evidence. A change is distinct from a check. Failed,
cancelled, unknown or ordinary shell results cannot prove a passed check. A known
failed attempt can be recorded as an inspection, preserving useful diagnostics,
but cannot resolve unfinished work without new successful evidence.
Successful reads may accompany a change or check as supporting context. At least
one primary mutation or recorded passed check must prove the entry; a new read
cannot resolve unfinished work using only an older primary result.
Jecode carries previous constraints, unfinished descriptions and completed
work forward even if the proposed memory omits or paraphrases them. Reusing an
old evidence reference cannot replace the completed entry's kind or identity.
Copying a shortened active preview restores the retained original description. Removing
an unfinished item requires its exact description and new successful evidence;
withdrawing a constraint requires a new user decision. Original
user requests and Jecode's recorded execution facts accompany the memory so that
continuation does not depend on the summary alone.

Summary requests label previous constraints and remaining items with scoped,
content-based references. Labels stay stable when other items are removed or
reordered. The model can cite an exact label to resolve an item instead of retyping
its exact wording. Jecode restores the original description before validation
and storage; the labels do not change the persisted history. Invented or obsolete
labels cannot resolve or withdraw old work. Jecode omits those contributions,
retains the original items and records a warning with an explicit reconciliation
action. Source and check validation still apply to every accepted entry.

Memory space takes priority over older recent groups. The saved validated ledger
retains every completed identity and evidence reference. If that ledger exceeds
the active allowance, its model view shows at most 16 recent completed entries,
with bounded descriptions, and a native `completed_archive` reference and count.
`history:memory` reads the complete current ledger with normal pagination.
Constraints, unfinished descriptions and resolutions keep their exact text in
the active view; they are never archived to hide a capacity failure. If the required
memory still cannot fit, Jecode preserves the previous context and reports it.
The memory allowance derives from the active input budget and carried summary
size; there is no fixed 8 KiB upper limit. Recent groups yield room for carried
facts and growth, while the complete projected request must still fit the model.
When the accepted memory is smaller than its reserved allowance, recent complete
groups can reclaim that space if the request still fits and remains smaller than
the previous view. Reserving future memory does not permanently discard a recent
tool preview that fits beside the actual summary.

Source-labelled transcript records, with previews for large outputs, are
processed in portions when they cannot fit a summary request. Summary requests
adapt their input portion and output allowance to provider context/truncation
errors. A provider context rejection can reduce the learned capacity and trigger
compaction even when catalog metadata is unavailable.

The memory must pass schema, source-reference and continuity checks, reduce the
request view and be saved before work continues. A rejected memory gets one
repair attempt with the rejected proposal and a specific reason. An unusable,
interrupted or unsaved memory leaves the previous view available and retains the
source history. These checks cannot establish semantic coverage of every user
requirement; the original transcript and saved output remain the evidence. A provider
output limit on an ordinary response still reports an incomplete response rather
than executing partial calls. The retained session can be continued explicitly.

The TUI reports compaction and completion in the conversation. Compaction and
retries are part of the original turn: its activity timer does not restart, its
draft/queue are not submitted, and its tools are not replayed. Resume restores
the saved context view and original transcript without starting work.

`read(path="history:N")` retrieves an original message by its zero-based index.
Tool results are returned as structured text and provider reasoning fields are
excluded. `history:requests` retrieves the complete ordered user requests, and
`history:memory` retrieves the full retained operational ledger.
All support the same line and UTF-8 byte pagination as file reads, mask known
credentials and remain read-only after compaction and explicit resume.
Tool-result reads include `request_history_reference` to retrieve their original
arguments, including exact content from a previous write without duplicating it
in version receipts.

## Foreground commands and output

Explicit file constraints use `protect` to register exact originals before
mutations. Native status survives reads, model summaries and resume. A proposed
final response with violated or unknown protections triggers the bounded review;
if they remain unresolved at its next final response, the turn returns an
incomplete-completion error and does not display that response as a successful
completion. A registered `require_check` condition similarly requires a passed
recorded check after the last tracked mutation. Re-reading a changed file or
writing different content cannot dismiss these conditions. This stronger condition
applies to registered protections; ordinary observation still uses the completion
notice described below.

When a new user request carries earlier protections, project writes and Bash
wait for a fresh preservation-scope review. `protect status` supplies a bounded
inventory of related unregistered files. The model must compare it with the new
request and classify newly covered paths before retrying. Earlier directory
declarations require recording their existing new members; status exclusions
cannot waive that scope. Files remain editable during their creating request.
Other candidates can be explicitly excluded with a reason. A status call alone
does not approve unclassified candidates. A refusal with
`outcome:"not_started"` means the command or write did not execute; an unresolved
scope refusal blocks successful completion and survives resume. Read-only file
inspection and session-temporary writes remain available during that review.

`protect restore` uses a saved binary baseline and the current-version token to
restore incidental changes exactly. Recovery uses synced registration intents,
including when a file was later deleted; it does not expand a saved directory
scope again. A missing original or corrupt baseline remains unknown. Registration
interrupted before durable metadata cannot silently capture the current bytes as
a replacement baseline. Such uncertainty requires a newer explicit user decision
before release and renewed work. See [TOOLS.md](TOOLS.md) for actions and scope.

Before repeating an exact Bash command whose native result was archived out of
the active transcript, Jecode returns `not_started` with its original history
reference. Review that result and the current request. A deliberate new check
after edits can supply `repeat_reason`; an explicitly once-only operation must
not be repeated. This is an execution review, not a cached fresh check or an
atomic exactly-once guarantee. Reworded scripts are outside this exact-text guard.

Jecode starts Bash with `errexit` and `pipefail` by default, including when the
model omits a verification flag. Verification commands use `check:true` to also
record `check_status` as passed, failed, cancelled,
timed_out or unknown. An unhandled pipeline failure therefore cannot be hidden
by a successful trailing `tail` or `echo`. Bash's normal conditional and explicit
error-handling rules still apply; negative tests must capture and assert their
expected exit codes. Explicit `check:false` selects ordinary shell behavior;
`shell_mode` reports which mode actually ran.

Recorded facts retain the latest tool, last tracked project mutation or uncertain
file comparison, up to four recent checks, and observed files still needing
inspection independently of model-generated memory. A later project write/edit
or observed indirect change makes earlier checks stale. A check that changes an
observed file is itself stale, even when its exit code is zero. Changes found
before a check are part of that check's input; changes found during it are not
treated as independently verified. These facts accompany the model request before
and after compaction. Request previews limit change details while full receipts
remain in the original history and saved session.

Files read, written or edited through project file tools are observed automatically;
`bash` can add existing project files with `watch`. Content fingerprints are read
in bounded chunks before and after commands, without trusting only size or
modification time. They use Rust's standard noncryptographic hasher for local
change detection, not security attestation. Unreadable, out-of-scope, cancelled
or unstable comparisons remain explicitly incomplete. Tracking restores versions
from native results on resume and resets with a new session. Original read history
can help restore a file when that read contains the required complete text.

A read marks inspection while retaining the observed indirect change for the
completion review. It cannot silently approve altered protected content. An
explicit write/edit or returning to the first observed version removes that
indirect-change reminder. Review acknowledgement remains tied to the same change
receipt across turns and resume; a later mutation needs review again. These facts
record actions, not proof of every constraint. If the model proposes a
final response while observed changes or incomplete comparisons still need
inspection, or the latest check failed or is stale, Jecode requests one completion
review in the same turn. Its instruction stays available while the model inspects,
restores incidental changes or checks again. Jecode retains the provisional
response in original history, records the review and keeps normal cancellation
and persistence. The review respects the user's scope and forbids replaying
completed formatters or diagnostic stages. It asks the model to compare affected
existing success, error and boundary behavior with the original inspected source,
including output format and exit status. The replacement final answer must be
self-contained; the provisional answer is retained as history, not delivered as
the final result. This guidance does not make the review a semantic oracle.
If facts still need attention at the
following final response, Jecode returns control with a separate completion notice;
it does not start an endless review loop or fabricate a process failure. A passed
check means its command exited successfully, not coverage of every requirement.
The inspection notice names the missing file-tool receipt; it does not assert
that no inspection occurred through Bash, whose command semantics are not inferred.

This is scoped observation, not a filesystem sandbox or continuous watcher.
Unobserved files, newly created paths not added through file tools, transient
changes restored within a command, concurrent changes after comparison and
external paths are not covered. Legacy sessions without native version receipts
start observation when a file is read or explicitly watched again. Bash retains
normal system access. See [TOOLS.md](TOOLS.md) for fields and usage.

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
invalid memory repair/rejection, large recent-group previews, history retrieval,
ordered user corrections, empty responses and strict check pipeline failures,
observed indirect file changes, stale mutating checks, content changes preserving
size and timestamp, deletion/restoration, explicit watch scope and completion notices,
complete output retrieval, long UTF-8 lines, sessions over 64 MiB, legacy migration
and crash recovery. Process tests cover cancellation and timeout after the shell
has exited; Windows and Linux tests also cover owner disappearance. Streaming
journal tests cover bounded growth, UTF-8 replacement and torn checkpoints. These
exercise the mechanisms; they are not a multi-day live OpenRouter soak test.
Preservation tests cover exact binary/CRLF restoration, deleted files, stale
restore tokens, refusal of same-request release, reads after violation, fresh
check requirements, final comparisons, compaction and resume. Saved started-tool
checkpoints simulate interrupted registration before its receipt, with complete
and partial copies, absent metadata and changed originals; they do not force-kill a process
during a filesystem copy.
The observed-file and completion-review runtime checks were executed on Windows
in this iteration; they have not been executed on Linux or macOS. Existing
Windows and Linux coverage includes the earlier recovery and persistence checks.
Linux checks execute under WSL with native tmpfs fixtures; macOS has all-target compilation coverage for
Intel and Apple Silicon, with native runtime checks still required.

The isolated native Windows console probe verifies visible recovery status,
compaction, a queued local command, a retained draft, width/height changes and
JSON export together. It uses loopback HTTP and synthetic credentials. Run from
the repository root in an interactive Windows terminal:

```powershell
cargo test --locked --offline tui::long_work_smoke::long_work_windows_smoke -- --ignored --exact --nocapture --test-threads=1
```
