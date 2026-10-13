# Tools and long-running work

Tools execute after a complete, validated model response; partial streamed
arguments never execute. Tool errors return to the model as results so it can
correct a call.

## Tool arguments

| Tool | Arguments | Behavior |
| --- | --- | --- |
| `read` | `path`, optional `offset`, `limit`, `byte_offset` | Reads UTF-8 text with line numbers and original line endings. Offset starts at 1; default 200 lines, maximum 2,000 per page. Follow `next_offset`, or `next_byte_offset` for a split long line. Also accepts saved Bash output and read-only history references. |
| `write` | `path`, `content` | Creates a file or replaces its contents. The parent directory must exist. |
| `edit` | `path`, `old_text`, `new_text` | Replaces one exact, nonempty occurrence. Missing or ambiguous matches leave the file unchanged. |
| `bash` | `command`, optional `timeout_seconds`, `wait_ms` | Runs an ordinary Bash command without an imposed timeout; an explicit positive timeout is an execution deadline from command start. Without `wait_ms` it runs in the foreground and returns exit/cancellation status, byte counts, the last 64 KiB of each stream and references to complete saved output. With `wait_ms` it starts a persistent process and waits up to that long; see [Persistent processes](#persistent-processes). |
| `process` | `action` (`list`, `poll`, `stop`), `process_id`, optional `wait_ms` | Lists this conversation's persistent processes, polls one for its status and new output, or stops it with its descendants. |

## File paths and paged reads

File tools resolve paths inside the working directory, including existing symbolic
links. Text files have no fixed file-size ceiling. Read pages contain up to 64 KiB;
long lines have a continuation byte offset. `total_lines` is available on reaching
EOF in line mode. Byte offsets seek directly and return unnumbered text with a
byte range. Binary or invalid UTF-8 pages return errors. Writes use a sibling temporary
file and rename.
Read-only files are rejected; project parent directories are not created automatically.

`history:N` reads the original message at zero-based index N, and
`history:requests` reads all user requests in order. `history:memory` reads the
current conversation summary. These references support pagination and
remain available after compaction and explicit resume. A `context_truncated`
tool result is a preview; use its `history_reference` to retrieve the full
original result. Provider reasoning fields are excluded from history reads.
Reading a tool result also returns `request_history_reference` for its original
tool call. Follow it for original arguments, including the exact content supplied
to `write` when the result itself contains only a byte count.
Indexed history pages also include `source` metadata: the original role,
`request_history` for the originating user request, `call_history` for the
matching tool call and the tool name. A result belongs to the request that issued
its call, even if another user message arrived before the result. Missing origins
are `null`. Metadata and content use credential redaction.

`attachment:ID` reads a file attached to the conversation, including one from
outside the working directory: text pages, an image or PDF shown again in the
next request, or the local path of an uninterpreted binary. Attachments are
read-only. See [ATTACHMENTS.md](ATTACHMENTS.md#reading-attachments-again).

## Temporary working files

The same file tools accept `tmp:relative/path` in the session's working area under
`~/.jecode/tmp/FOLDER_BUCKET/SESSION_ID/`; temporary writes create their parent
directories. Absolute paths inside that area are accepted too. Bash keeps the
project working directory and receives `JECODE_TMP`, `TMPDIR`, `TEMP` and `TMP`
for disposable files. Working files survive interruption, exit and explicit
resume. `/new` uses a new area; `/tmp clean` clears only the current one when idle.
Deleting a session from `/resume` removes its entire owned area.
There is no automatic eviction. Keep durable project verification in the project;
use the session area for one-off checks and their disposable output. See
[TEMPORARY.md](TEMPORARY.md) for placement, cleanup and tool usage.

## Bash execution and saved output

Bash has normal system access; the working directory is not a shell sandbox.
It runs without startup profiles, `BASH_ENV`, `ENV` or `OPENROUTER_API_KEY`, with
closed stdin. Complete streams are saved under this folder's session bucket in
`~/.jecode/sessions/BUCKET/outputs/SESSION_ID/`. `stdout_file` and `stderr_file` are read-only
`output:SESSION_ID:OUTPUT_ID:stdout` / `:stderr` references accepted by `read`, including
after resume. Known configured credentials are masked before writing these files.
Older flat `output:OUTPUT_ID:stdout` / `:stderr` references remain readable;
their files are retained when deleting a session because they have no reliable owner.
`stdout_bytes` and `stderr_bytes` count observed bytes before decoding or masking;
per-stream truncation flags describe the bounded returned preview. Foreground
commands finish before the call returns; persistent processes, below, use the
same execution path. On Windows, an owned supervisor creates each command
inside a native Job Object and waits for its descendants. Cancellation, timeout
or the disappearance of Jecode closes that Job, terminating its members even
when the original shell has already exited. Failure to confirm completion is
reported as an unknown outcome. On Linux/macOS, an owned Bash guardian is
started before the command and watches a private pipe held only by Jecode.
Owner disappearance also stops the command group. Timeout and cooperative
cleanup use native `kill`; processes that leave the group are outside this
cleanup boundary. The TUI on all three platforms provides
cooperative stop-and-continue behavior with Ctrl+C/Esc and cleanup on Ctrl+Q.
Plain chat and single prompts retain native signal behavior; the guardian also
cleans up their command groups on exit. Interrupted tool outcomes are recovered
as unknown on resume.

## Persistent processes

`bash` with `wait_ms` starts the command as a persistent process: still its own
Bash process with closed stdin, no shared shell state, no PTY and no input
after start. Jecode waits up to `wait_ms` milliseconds for it to finish;
`wait_ms: 0` returns an immediate snapshot. The wait expiring never stops the
command. `timeout_seconds` remains a separate deadline measured from command
start; polling does not extend it, and there is no default timeout.

The result carries a `process_id` such as `process-…`, distinct from operating
system PIDs and unique across runs, plus a `status`:

| Status | Meaning |
| --- | --- |
| `running` | Still executing. No `exit_code` is reported. |
| `stopping` | A stop was requested and cleanup has not been confirmed yet. |
| `exited` | Finished; `exit_code` is present (`null` when the platform reported none). |
| `timed_out` | Reached `timeout_seconds` and was terminated. |
| `stopped` | Terminated by `process stop` or by conversation cleanup. |
| `unknown` | Completion or cleanup could not be confirmed; `error` explains it and `outcome` is `unknown`. Inspect the workspace before repeating the command. |

`process` takes an `action`:

- `list` returns every process of the current conversation runtime with its
  `process_id`, `status`, `command`, `stdout_file` and `stderr_file`. It does not
  consume output and is how the model recovers IDs after context compaction.
- `poll` returns the status and the output written since the previous result for
  that process, waiting up to `wait_ms` (default 0) for it to finish.
- `stop` terminates the process and its descendants within the native boundary
  described above and waits up to 10 seconds for the final status. If that wait
  expires or is interrupted, the result may still be `stopping`; use `poll`
  to confirm completion. Interrupting the wait does not undo the stop request.

Each `bash`/`process` result for a persistent process reads both saved streams
from a per-process cursor. For each stream it returns `<stream>` text,
`<stream>_offset` (the saved-file byte where that text starts),
`<stream>_next_offset` (where the next result continues), `<stream>_skipped_bytes`
(unread bytes left out because only the last 64 KiB of new output is returned)
and `<stream>_file`. Skipped bytes stay readable with `read` and `byte_offset`.
Previews start and end on UTF-8 boundaries: a cut skips a partial leading
character, and while the process runs an incomplete trailing character waits
for the next result. Status is read before output, so a final status guarantees
the saved streams are complete; the returned preview remains bounded. A failed
stream read leaves both cursors unchanged. Saved files contain only
credential-masked text; while a process runs, a trailing fragment that could
begin a configured secret is held back until the next bytes show whether it
needs masking.

Processes belong to the conversation runtime, not to a turn. They survive model
calls, tool calls and turns, and a finished process stays listed with its final
status; there is no automatic eviction. Stopping a turn while a foreground
command runs still terminates that command. Stopping it during a `wait_ms` wait
or a `poll` only ends the wait: the result includes `wait_cancelled: true` and the
process keeps running. If the turn is stopped before launch completes, the call
reports either that nothing started or an unknown outcome with output references.

`process stop`, app exit, a successful `/new` or resume of another session, and
deleting the live session stop every process of the conversation. A failed or
cancelled switch keeps them. Saved output and history remain readable afterward.
Resume never restarts commands: earlier IDs return an error that points to
`list`, and historical tool cards keep the status recorded when they returned.
Jecode does not wake the model when a process changes; the model polls when it
needs the state.

### Viewing processes

`/terminal` lets you inspect the same processes without involving the model.
It is read-only: it cannot start, stop or send input to a process, and it never
calls `process poll`, so the model's next result still begins at its own
cursor. Output comes from the same credential-masked files, read in bounded
windows of at most 64 KiB each.

In the TUI, `/terminal` opens a panel immediately, even while the model works,
listing active processes first, then finished ones, with their command, status
and exit code. Enter opens the selected process's output; Tab switches between
Output (stdout) and Errors (stderr), which are separate saved streams. The view
follows new output at the bottom; scrolling up pauses it at that position and
loads older output as needed, and End resumes following. Esc returns from the
output to the list and closes the list; Ctrl+C does the same and never stops the
process. `/terminal ID` opens one process's output directly. Finished processes
remain viewable until the conversation ends; switching conversations closes the
panel.

In plain chat, `/terminal` prints the process list and `/terminal ID` prints a
process's status with the last 64 KiB of each saved stream. Neither adds
anything to the conversation or its history.

## Long-running turns

There is no fixed turn duration, model-request count or tool-batch count. The model
can continue calling tools until it returns a final response or you stop the turn.
Temporary connection/provider failures retry automatically until recovery or
interruption. Automatic compaction uses the selected model to preserve continuity
while keeping original messages for display, resume and export. The active request
uses the model's context capacity rather than a fixed 4 MiB transport ceiling.
Malformed or incomplete responses do not execute tools. Permanent provider or
storage errors return control to you with the session retained. Read
[LONG_WORK.md](LONG_WORK.md) for recovery, context and resource behavior.
