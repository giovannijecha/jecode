# Saved sessions

Jecode autosaves conversations in `~/.jecode/sessions/`. `JECODE_HOME` moves this
store together with the personal configuration. Each conversation belongs to
the canonical working directory in which it started. Directory aliases resolve
to the same scope; another directory cannot list or resume it. Moving a project
to a new path is not a session migration in this version.

## Start and resume

`jecode` opens a fresh conversation. When this folder has saved conversations,
Jecode shows a reminder to use `/resume`. Opening Jecode never resumes work
automatically.

A conversation is saved from its first sent request, before provider work starts.
Adding an attachment to the composer or another pending draft also saves the
session so its copied bytes remain reachable. Opening the app, text-only
drafting, queueing and local commands alone do not create a saved session.
An interrupted or failed request still leaves a saved conversation. Before
the first request or attachment, unsent input stays in memory and is lost on exit.

```powershell
jecode resume
jecode resume SESSION_ID
jecode --plain resume SESSION_ID
```

After a successful save, the exit summary prints `jecode resume SESSION_ID`.
Run that command from the conversation's original project folder.

`/resume` opens the same folder-scoped selector inside the TUI. `/resume ID`
selects a saved conversation directly. The list includes the current conversation
after its first sent request or attachment and marks its row `current`. An empty
launch, text-only unsent draft or local commands alone never add a current row. Selecting that
row returns to the existing composer. Saved sessions are ordered by most recent
update, with their first request as the title, age, model and identifier. A
legacy draft-only session uses its draft as the title. No model call generates titles.

The selector replaces the composer. Up to eight options use the numbered menu;
larger lists use the existing searchable menu. Arrows and Enter select, 1–8
choose a numbered option, and Esc closes it. Ctrl+D marks the selected row for
deletion (see below). Its search text remains separate from the draft. An already
open session reports an error instead of creating a
second writer.

Resuming restores the conversation, tool previews, local command results,
model, effort and prompt history. It uses the current configured OpenRouter
credential. `/settings` defaults remain the defaults for `/new`; they are not
changed by a resume. No provider request or tool execution occurs on resume.

`/new` saves the preceding conversation and creates an independent session with
the saved defaults. Unsent work moves with the active composer and pending
drafts. The viewport and its source layout reset to the new conversation.
Earlier saved conversations remain available through `/resume`; provider context
and exports use the active
session independently of its terminal layout.

## Delete

In `/resume`, press Ctrl+D on the selected row to mark it for deletion. The row
shows `Delete?` and the same list shows the confirmation controls. Enter confirms;
Esc/Ctrl+C or Ctrl+D cancels the mark while keeping the list open. A second Esc
closes the list and keeps the draft. Moving to another row or editing the search
also cancels the mark; numbered shortcuts never confirm deletion.

Every successful deletion refreshes the same list with its search intact, so
further conversations can be deleted without reopening it. This includes the
current or last saved conversation. An empty list shows `No saved conversations`
and remains open until Esc. Plain chat accepts
`d NUMBER` or `d ID` inside `/resume` and requires typing `delete` to confirm;
Enter or end of input cancels. No separate slash command or TUI menu is needed.

Deletion removes the selected conversation's journal, legacy snapshots and
backups, recovery copies, listing cache, temporary working area and owned tool
outputs. It holds the same exclusive session lock as a writer and rejects a
session open in another process. The small `.lock` file remains so concurrent
lockers cannot acquire a replacement file. Project files and manual exports
remain outside this operation. Older flat output files have no reliable session
owner and remain readable; references kept for that reason are reported.

Deleting the current conversation opens a fresh one using the saved model/effort
defaults. Its model context and prompt history are cleared. The TUI clears its
owned conversation viewport; the shell screen and scrollback remain intact.
Other saved sessions remain available. The session
list stays open over the fresh context; Esc returns to its composer. The unsent
composer, FIFO queue and paused drafts stay with the new conversation. The new
conversation is saved after a request is sent or its inherited drafts contain
attachments. Old autosave handles cannot
recreate deleted history.

During a turn, `/resume` joins the existing command queue. A stopped or failed
turn leaves unsent commands as separate paused drafts for review. Confirmed file
removal runs in the background with the selected row showing `Deleting…`; the list
ignores selection/editing until removal finishes. It runs to completion,
including when quitting with Ctrl+Q. Ownership and file types are checked before
removal. Foreign or unreadable legacy records block deletion and stay unchanged.
A filesystem failure can leave some ancillary files removed; the last valid
conversation record is removed last, and the error stays visible for retry.
If final output/temp directory removal fails after unlinking its ownership record,
Jecode attempts to restore that record without overwriting another entry. Failure
to restore ownership is reported explicitly.

## Unsent work and interruption

The composer text and Unicode-safe caret, up to eight queued messages, separate
paused drafts with their own carets, and up to fifty dispatched prompts are saved
once the conversation has a sent request or an attachment in its input. Prompt history excludes local commands,
including commands in older saved histories. Ctrl+P/N browses sent prompts and
preserves the original unsent composer. Submitting a recalled prompt restores it;
editing a recalled prompt also saves that edit as a separate paused draft if
Jecode closes before submission.

On resume, unsent queued messages become separate Paused entries, oldest first;
the composer keeps its own text and caret. A legacy withdrawn edit is also
recovered as its own paused entry while the prior composer is restored. Repeated
resume keeps entries distinct, even when their text is identical. Open `/drafts`
or press Alt+Up to review them. Enter edits one in place; Ctrl+S explicitly
sends or queues a paused draft. No paused message or local command runs
automatically after resume or an interruption.

The line-based `--plain` chat restores the conversation and displays the
composer draft and each paused draft separately for copying/editing. Paused
drafts remain in the saved session for review in the TUI; plain mode has no
draft selector or prefilled multiline editor. All three platforms open the
multiline TUI by default when both input
and output are interactive.

If the initial checkpoint fails, the turn stays ready and the request is not
added to the saved conversation. The TUI keeps it and any following queued
messages as separate paused drafts without executing them. Restore storage
access, then review and send each intended message explicitly.

Completed tool results retain their original payloads. A process exit after a
tool's saved start but before its saved result leaves an **unknown outcome**:
the command may already have changed files. Recovery records this as a tool
error. Calls which had not started are marked **not executed**. Missing results
are paired with their calls so the next model request has a coherent protocol.
Recovery never repeats either kind of call.

Partial response text is retained as incomplete display evidence, separate from
completed provider messages. An interrupted conversation reopens ready for a
new request, with the interruption recorded in the journal and export. The TUI
restores partial text without replaying historical UI warnings, errors or local
command output. Its new resume feedback is temporary; plain mode retains its
command output.

## Storage contract

Disposable working files live separately in `~/.jecode/tmp/FOLDER_BUCKET/SESSION_ID/`.
Resume reuses the same area; `/new` selects a different one without deleting old
working files. `/tmp` inspects it and `/tmp clean` explicitly clears it when ready.
Cleanup intent and outcome are retained as `temporary_cleanup` events. An
interrupted cleanup is not replayed. The latest status is supplied in subsequent
model requests independently of the compacted summary. See [TEMPORARY.md](TEMPORARY.md).

Complete tool streams use marked `outputs/SESSION_ID/` directories inside the
folder's session bucket. References use `output:SESSION_ID:OUTPUT_ID:stdout` or
`:stderr`. `read` also accepts older `output:OUTPUT_ID:stdout` / `:stderr` paths
in the shared output root. This compatibility does not establish ownership of
those old files. `/tmp clean` keeps all tool streams; session deletion removes only the
selected session's marked output directory, including unreferenced crash remnants.

Attached files live in the bucket's `attachments/` pool and are referenced by
id from messages and saved input. An asset is removed when no saved session
refers to it any more; see [ATTACHMENTS.md](ATTACHMENTS.md#storage-and-retention).

New sessions use an owned append journal, `jecode.session.journal`, version 3,
in `SESSION_ID.jsonl`. Files live under a stable folder bucket. The saved canonical directory is checked
independently of the bucket name. Sessions contain timestamps, directory,
model/effort, provider messages, local events, the provider compatibility
boundary, the compacted context view, unfinished-turn state and unsent input. They contain no configuration
credential; configured keys are masked in recorded content.

The agent writes checkpoints before provider work, after validated responses,
before each tool and after each result. Turn completion/interruption and local
command results are also saved. The first streamed text is saved immediately;
subsequent streaming and TUI edits use a 250 ms save interval. A clean TUI exit
waits for active work to stop and flushes input. A forced exit can lose the most
recent unsaved edits or streaming tail.

Each synced newline-terminated transaction appends new provider messages and
local events together with changed metadata and input. Partial streaming text
uses UTF-8-safe prefix and suffix increments rather than rewriting its accumulated
content at every checkpoint. Unchanged summaries and input are omitted from the
transaction. Previous conversation messages are not rewritten or removed.
A failed write attempts to restore the preceding acknowledged journal length;
an unsuccessful rollback requires reopening the session. A torn final transaction is recovered from
the preceding complete checkpoint; its incomplete bytes are preserved separately
before repair. Corrupt complete transactions are reported and left unchanged.

Existing version 2 journals remain readable and accept version 3 continuations
without rewriting their earlier transactions.
Existing version 1 `.json` snapshots and their `.json.bak` backups remain readable.
On explicit resume they are migrated to a new journal while the original files
remain in place. A damaged legacy primary is preserved separately when recovering
its valid backup. Entries with both formats appear only once in the picker.
If migration stops before the journal's first complete checkpoint, listing and
resume can still read a valid legacy snapshot or backup. Resume preserves any
incomplete journal bytes before writing its initial checkpoint. Complete invalid,
foreign or unsupported journal records never fall back to the legacy file.
Unsupported format versions, foreign-folder documents and mismatched identities
are reported and left unchanged. There is no fixed 64 MiB session ceiling. Save errors remain visible,
and a failed checkpoint before tool execution prevents that tool from starting.

An OS file lock, held through Rust's standard library, permits one writer per
session. Lease release explicitly unlocks it, including Unix descriptors duplicated
by a concurrent fork before exec. Process exit releases it; the small `.lock` file may remain. Incomplete
temporary files are not session entries. There is no automatic deletion or
retention policy.

`SESSION_ID.summary.json` is a small, optional listing cache, refreshed after a
committed save. It contains the folder, bounded title, model, update time and
journal length/modification fingerprint. A valid cache lets the picker avoid
reading the complete journal. Missing, stale or invalid caches fall back to the
journal and are rebuilt; cache-write failures do not fail a committed checkpoint.
Explicit resume always reads and validates the journal. Conversation reconstruction
indexes local events and tool calls, then traverses messages once.

`/export` remains a manual `jecode.conversation` JSON snapshot in the working
directory. It includes the original provider conversation, recovery evidence and
compaction summaries;
it does not include unsent input and is not the resume format. Session import,
cross-folder resume and execution while Jecode is closed are not implemented.
Automatic compaction affects the active model context, while the original
conversation remains saved. See [LONG_WORK.md](LONG_WORK.md).

## Verification

Automated Windows and Linux checks cover folder boundaries, explicit resume,
model transitions, retained reasoning fields, tool/result pairing, input/queue
recovery, read-only saves, corrupt-file recovery, future formats and
exclusive writers, legacy migration, version 2/3 continuations, incremental UTF-8
stream recovery, failed preparation/retry, listing caches, linear reconstruction,
first-request saving, confirmed current/inactive deletion, retired autosave
handles, empty-current exclusion and current/last deletion keeping the list open,
lock conflicts, owned output/temp removal and preserved project files,
torn journal tails, interrupted first migrations, and histories exceeding
the preceding 64 MiB ceiling. A subprocess test performs an actual file write and is killed
before saving its result, then verifies lock release and unknown-outcome recovery.

The isolated interactive fixture closes and reopens the native TUI, restores a
queued local command and draft, resizes the resumed conversation, and exports
the restored evidence. It uses loopback HTTP and synthetic credentials. Run from
the repository root in an interactive Windows terminal:

```powershell
cargo test --locked --offline tui::persistence_smoke::sessions_windows_smoke -- --ignored --exact --nocapture --test-threads=1
```

Case-insensitive folder aliases and the manual Windows console fixture
have Windows-specific checks. Shared TUI selectors, editor, queue and persistence
checks also execute on Linux. Linux checks execute under WSL with a musl build
and native tmpfs fixtures. macOS Intel and Apple Silicon have compilation
coverage; native runtime checks remain outstanding.

The Linux PTY fixture in [testing guide](../development/TESTING.md#native-terminal-checks) also exits with Ctrl+Q,
reopens the session, verifies Unicode multiline draft recovery, and confirms that
Ctrl+C clears input without exiting. It stops a real foreground command and
preserves its interrupted result without replaying queued work.
