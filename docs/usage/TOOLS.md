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
| `protect` | `action`, optional `paths`, `reason`, `scope_exclusions`, `expected_current`, `require_check` | Registers exact preservation baselines and directory declarations, reports their current state, restores one file or releases a protection after a newer user decision. Violated or unknown protections prevent successful completion. |
| `bash` | `command`, optional `check`, `repeat_reason`, `watch`, `timeout_seconds` | Runs in the foreground without an imposed timeout. Errexit and pipefail are the default; `check:true` also records a check verdict, and explicit `check:false` selects ordinary shell behavior. Archived exact commands require review before repeating. `watch` observes existing project files before execution. An explicit positive timeout is supported. Returns exit/cancellation status, observed file changes, byte counts, the last 64 KiB of each stream and references to complete saved output. |

## File paths and paged reads

File tools resolve paths inside the working directory, including existing symbolic
links. Text files have no fixed file-size ceiling. Read pages contain up to 64 KiB;
long lines have a continuation byte offset. `total_lines` is available on reaching
EOF in line mode. Byte offsets seek directly and return unnumbered text with a
byte range. Binary or invalid UTF-8 pages return errors. Writes use a sibling temporary
file and rename.
Read-only files are rejected; project parent directories are not created automatically.

## Explicit file preservation

Before mutations or formatters, `protect(action="record", paths=[...], reason=...,
require_check=true)` registers the user's explicit preservation requirements.
Paths must be existing project files or directories; directories expand to the
regular files present at registration and retain a directory declaration.
New files stay editable during their creating request. In later requests,
`covered_by_recorded_directory` candidates must be recorded before project
operations; status exclusions cannot discard that declaration. The declaration
survives compaction and resume, including supported legacy directory receipts.
It does not capture new baselines automatically or infer constraints from names.
Register only files explicitly required to remain byte for byte unchanged.
An explicit request to edit or delete a file permits that change. Do not register
its target as a preservation baseline or treat `record` as a backup step.
Use the ordinary file or Bash tools when no carried protection blocks the operation.
Preserving an API, function signature or supported behavior does not make its
source file immutable; leave source needed for the requested change editable.
Direct project `write` and `edit` calls refuse protected paths. Bash retains its
normal access; native comparisons after tools and before completion detect
remaining changes independently of model memory.

`protect(action="status")` reports all protections. States are `preserved`,
`violated`, `unknown` or `released`. Tool results include `file_protections`;
`file_protections_scope:"all"` denotes an authoritative list, while other tools
return changed entries. A receipt includes the original `baseline`, `snapshot_id`,
registration request, reason, check requirement and history reference. The latest
`expected_current` token identifies the version to restore; deleted files use
`"missing"`.

Project writes and Bash with active earlier protections require a fresh review
for each new user request. A status call alone does not approve unclassified files.
Results include `scope_review` with `related_unregistered_files`: a bounded
inventory under non-root parents of active registrations and explicitly declared
directories, capped at 64 candidate
files and 512 visited or queued paths. `related_inventory_complete:false` means
the model needs further project inspection. The inventory does not infer user
constraints or automatically make candidate files immutable. Record covered
files, or use status with `scope_exclusions` and a nonempty `reason` for other
editable or uncovered files. Exclusions accept at most 64 existing individual
project files and cannot waive an active baseline or earlier directory declaration.
An incomplete inventory also needs inspection and a scope reason. Scope decisions
reset on each new request and rebuild from native receipts on resume. Root-level
file registrations do not trigger a whole-workspace scan. An explicitly declared
project-root directory uses the same bounded inventory.

Optional empty `scope_exclusions:[]` supplies no exclusions, including on other
actions. An empty status `reason` supplies no scope decision. Actual exclusions
still require `status` and a nonempty reason and cannot waive protected files.

Before that review, a refused operation returns `outcome:"not_started"` and has
no command or project-write side effects. Review the scope and retry the intended
operation; counting the refused attempt as a completed formatter is incorrect.
An unresolved refusal prevents successful completion, including after resume.

`restore` takes one registered path and that token. It verifies the private
baseline and current version, then restores the exact binary bytes through an
owned sibling temporary file. It preserves CRLF, final-newline state and existing
permissions, and refuses a newer version, redirection, invalid baseline or
read-only target. It can restore a deleted file when its parent still exists.
Do not reconstruct preserved files from text previews. Fingerprints use Rust's
noncryptographic hasher for local version checks; preservation comparisons also
stream both files byte for byte. This is not an atomic lock against concurrent
filesystem writers.

Reading, writing and re-recording cannot replace an active baseline or approve a
violation. `release` requires one path, a nonempty reason and a newer user request
explicitly permitting the change. The native guard checks request order; the
model interprets the user's permission. `require_check:true` also requires a
passed recorded check after the last tracked project mutation. It does not prove
that the check covers every acceptance criterion.

Releasing a directory declaration permits reconsidering newly covered candidates;
its individual file baselines remain active and need their own authorized releases.
It cannot be released during its registering request.
An authorized individual release is an exception for that member; it does not
discard the directory declaration or cover other newly discovered members.

Baselines and registration intents are private, exact, unredacted data stored in
the session's owned output directory, separate from the project and disposable
temporary files. They survive compaction and resume and are removed with their
owned session. Do not use them to copy unrelated sensitive data. A durable intent
retains original versions if registration is interrupted before its tool receipt.
Incomplete copies can be recreated only while the declared original still exists.
An absent or corrupt intent leaves an unknown registration and blocks mutations
and re-recording. Releasing its exact incomplete-registration marker from status
requires a newer user decision accepting the unavailable original version.
Intents have a local integrity stamp to detect partial or changed metadata;
this uses the standard noncryptographic hasher, not security authentication.

Natural-language preservation requirements still require the model to register
the relevant paths. Protections are scoped preservation, not a shell sandbox or
a guarantee about unregistered files.

`history:N` reads the original message at zero-based index N, and
`history:requests` reads all user requests in order. `history:memory` reads the
complete retained operational ledger, including completed work omitted from its
bounded active view. These references support pagination and
remain available after compaction and explicit resume. A `context_truncated`
tool result is a preview; use its `history_reference` to retrieve the full
original result. Provider reasoning fields are excluded from history reads.
Reading a tool result also returns `request_history_reference` for its original
tool call. Follow it for original arguments, including the exact content supplied
to `write` when the result itself contains only a byte count and version receipt.
Indexed history pages also include native `source` metadata: the original role,
`request_history` for the originating user request, `call_history` for the
matching tool call, and `eligible_proof` kinds from the memory validator. A result
belongs to the request that issued its call, even if another user message arrived
before the result. Missing origins are `null`; reading old evidence supplies no
new write or passed-check receipt. Metadata and content use credential redaction.

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

Exact Bash commands with a native result archived outside the active transcript
are refused with `outcome:"not_started"` and `previous_history_reference`.
Review the original result and request before repeating. A deliberate rerun, such
as a check after edits, needs a nonempty `repeat_reason`. Unknown outcomes need
investigation. This does not return a cached fresh check, normalize shell scripts
or provide atomic exactly-once execution; changed command text is not covered.
An optional empty `repeat_reason` is treated as omitted: it does not block a
first execution and cannot authorize an archived command to run again.

Use `check:true` for verification and run one check per command. Unhandled
pipeline failures propagate by default, including without the flag. Explicit
`check:false` restores ordinary shell behavior; `shell_mode` records the mode.
Negative tests must explicitly capture and assert expected nonzero
exits. `check_status` distinguishes passed, failed, cancelled, timed_out and
unknown; ordinary commands report no check verdict. A passed command does not
prove that every requirement is covered. See [LONG_WORK.md](LONG_WORK.md) for
recorded checks and revalidation after source changes.

Project files successfully read, written or edited become observed files. Bash's
optional `watch:["tests/basics.rs","README.md"]` also observes existing regular
project files, including files inspected through Bash instead of `read`. Jecode
compares their content before and after commands. `file_changes` reports the
path, before/after state, first observed version, history references and whether
the file returned to that first version. The source interval distinguishes a
change already present before execution from one observed during execution;
it does not identify which process caused it. No content is copied into these
receipts, and there is no directory scan or Git dependency.

`file_tracking` names the scope, observed-file count and incomplete comparisons.
An empty list covers only observed paths. New unobserved files, transient changes
restored within a command, and changes made after its final comparison are outside
that observation. Temporary working files and saved output references are excluded.
Versions are restored from native tool results on explicit resume; legacy results
without version receipts require observing the files again. A file read or explicit
write/edit counts as inspection, not proof that a change respects the user's scope.
See [LONG_WORK.md](LONG_WORK.md) for execution facts and completion notices.

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
