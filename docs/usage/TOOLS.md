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
| `bash` | `command`, optional `timeout_seconds` | Runs an ordinary Bash command in the foreground without an imposed timeout; an explicit positive timeout is supported. Returns exit/cancellation status, byte counts, the last 64 KiB of each stream and references to complete saved output. |

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
There is no automatic eviction. See [TEMPORARY.md](TEMPORARY.md) for scope,
cleanup and tool usage.

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
per-stream truncation flags describe the bounded returned preview. Commands must
finish in the foreground. On Windows, an owned supervisor creates each command
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
