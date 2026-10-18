# Saved sessions

Jecode saves conversations under `~/.jecode/sessions/` (`JECODE_HOME` changes
the home location). Sessions belong to the canonical working directory where
they started. Run resume commands from that directory; moving the project to a
new path does not move its sessions. Opening `jecode` starts a new conversation,
even when saved sessions exist.

## Start and resume

```powershell
jecode resume
jecode resume SESSION_ID
jecode --plain resume SESSION_ID
```

`jecode resume` opens a folder-scoped picker. In the TUI, `/resume` opens the
same picker and `/resume ID` opens a specific session. The picker shows the
most recently updated sessions first, with the first request as the title,
model, age and ID. Once saved, the active session appears as `current`;
selecting it returns to the composer. A session already open in another
process cannot be resumed at the same time.

The exit summary prints `jecode resume SESSION_ID` after a successful save.
Resume restores the conversation, tool results, model, effort, routing, prompt
history and saved input. It uses the currently configured OpenRouter key and
does not rerun provider requests or tools. `/settings` defaults are unchanged.
Use `/new` to save the old conversation and start an independent one with those
defaults. The unsent composer and pending drafts move to the new conversation.

A session is saved from its first sent request, before provider work starts.
Adding an attachment to unsent input also saves it so the copied bytes remain
reachable. Opening Jecode, drafting text, queueing messages or running local
commands alone does not create a saved session. Before the first request or
attachment, unsent input is lost on exit. A failed or interrupted first request
still leaves a saved session if its initial checkpoint succeeded.

## Recover unsent work

The TUI saves the composer, caret, queued messages, paused drafts and sent
prompt history after a session has a request or attachment. On resume, queued
messages become separate paused drafts; the composer retains its own text.
Open `/drafts` or press Alt+Up to review them. Enter edits a draft; Ctrl+S
sends it explicitly. Nothing pending runs automatically after resume or an
interruption. In `--plain`, recovered drafts are displayed for copying and
editing, while the TUI remains the place to manage paused drafts.

If the initial save fails, Jecode does not start the provider request or a
tool. Restore storage access, review the retained drafts and send the intended
work again. If Jecode stops after a tool starts but before its result is saved,
its effect is **unknown**: inspect project files before deciding whether to
retry. Calls that had not started are marked **not executed**. Recovery does
not rerun either kind. Partial streamed text is retained as incomplete
evidence; the conversation reopens ready for a new request. A forced exit can
lose the latest unsaved draft edit or streaming tail.

Older version 1 snapshots and backups remain readable. Explicit resume
migrates them to the current journal while preserving the originals. A damaged
legacy primary can be recovered from a valid backup. An incomplete final
journal transaction is recovered from the last complete checkpoint, with the
incomplete bytes preserved separately. Corrupt complete records, unsupported
formats and records belonging to another folder are reported and left
unchanged. Save and recovery errors remain visible; retry only after checking
the reported files. See [architecture](../development/ARCHITECTURE.md) for the
journal format and transaction details.

## Delete

In the TUI `/resume` picker, select a row and press Ctrl+D to mark it. Enter
confirms; Esc, Ctrl+C or Ctrl+D cancels the mark. The picker stays open after
deletion, including when the last saved session is removed. In `--plain`, enter
`d NUMBER` or `d ID` in `/resume`, then type `delete` to confirm. A session
open in another process cannot be deleted.

Deletion removes that session's journal, legacy snapshots and backups,
recovery copies, listing cache, [temporary area](TEMPORARY.md) and owned
complete tool outputs. Attachments no longer referenced by another saved
session are also eligible for removal; see
[attachment retention](ATTACHMENTS.md#storage-and-retention). Project files and
manual [exports](EXPORT.md) remain. Older shared output files cannot be
reliably assigned to a session and may remain; Jecode reports retained
references.

Deleting the current session opens a fresh conversation with the saved
model and effort defaults. Its context and sent prompt history are cleared;
unsent composer text, queued messages and paused drafts are kept. The new
session is saved when a request is sent or inherited input contains
attachments. If deletion fails partway through, some owned files may already
be gone. Read the error and retry from `/resume`; Jecode removes the last valid
conversation record last. Foreign or unreadable legacy records block deletion
and remain untouched.

Saved sessions have no automatic retention period. Temporary files and their
explicit cleanup are described in [TEMPORARY.md](TEMPORARY.md); exports are
separate snapshots and cannot be resumed.
