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
most recently updated sessions first, each with the first request as the title
and a compact age (`5m`, `2h`, `3d`); typing filters by title, backend, model
or ID. The plain picker lists all of them on each row. Once saved, the active
session appears as `current`; selecting it returns to the composer. A session already open in another
process cannot be resumed at the same time.

The exit summary prints `jecode resume SESSION_ID` after a successful save.
Resume restores the conversation, tool results, backend, model, effort, routing,
prompt history and saved input. It uses the current credential for that backend
(the OpenRouter key or active ChatGPT account) and does not rerun requests or
tools. `/settings` defaults are unchanged.
Use `/new` to save the old conversation and start an independent one with those
defaults when their provider is available. Otherwise, it keeps the active
provider, model and effort. The unsent composer and pending drafts move to the
new conversation. Without a usable selection, an action banner points to
model or provider configuration and leaves the current conversation intact.

`/model` changes the active provider and model within the same conversation.
The session ID, original history, attachments, live processes and browser remain
available. The next request uses the new selection and resets its token
calibration. A failed selection save retains the previous backend and model.
Native response metadata stays attributed to its original backend and model;
only a compatible selection reuses it. Older sessions infer OpenRouter when
backend identity is absent, without rewriting the original messages.

A session is saved from its first sent request, before provider work starts.
Adding an attachment to unsent input also saves it so the copied bytes remain
reachable. Opening Jecode, drafting text, queueing messages or running local
commands alone does not create a saved session. Before the first request or
attachment, unsent input is lost on exit. A failed or interrupted first request
still leaves a saved session if its initial checkpoint succeeded.

## Recover unsent work

Sending a task before provider configuration is complete returns it to the
composer without saving an execution turn. Attachment drafts can still be saved
and recovered before a model is selected. They appear as **Not selected** in the
session list and store `model: null`, rather than a made-up model identifier.
Resuming such a draft uses the current selection, or keeps waiting for
configuration. Its text and attachment bytes remain intact.

The TUI saves the composer, caret, queued messages, paused drafts and sent
prompt history after a session has a request or attachment. On resume, queued
messages become separate paused drafts; the composer retains its own text.
Sent attachments stay in the conversation and prompt history; only unsent
input returns to the composer.
Open `/drafts` or press Alt+Up to review them. Enter edits a draft; Ctrl+S
sends it explicitly. Nothing pending runs automatically after resume or an
interruption. In `--plain`, recovered drafts are displayed for copying and
editing, while the TUI remains the place to manage paused drafts.

If the initial save fails, Jecode does not start the provider request or a
tool. Restore storage access, review the retained drafts and send the intended
work again. If Jecode stops after a tool starts but before its result is saved,
its effect is **unknown**: inspect project files and external state before
deciding whether to retry. In an independent parallel group, each completed
call is saved separately before its result appears in the interface. Recovery
keeps those known results and file diffs in original call order, even if they
finished out of order. Started calls without saved results remain **unknown**;
calls that had not started are marked **not executed**. Recovery does not rerun
any of them. A returned `process_id` may still have work running; poll it
before treating the command as finished. Partial streamed text is retained as
incomplete evidence; the conversation reopens ready for a new request. A forced exit can
lose the latest unsaved draft edit or streaming tail.

An incomplete final journal transaction is recovered from the last complete
checkpoint, with the incomplete bytes preserved separately. Corrupt complete
records, unsupported formats and records belonging to another folder are
reported and left unchanged. Session files from before the version 3 journal
are not read. Save and recovery errors remain visible; retry only after checking
the reported files. See [architecture](../development/ARCHITECTURE.md) for the
journal format and transaction details.

## Delete

In the TUI `/resume` picker, select a row and press Ctrl+D to mark it. Enter
confirms; Esc, Ctrl+C or Ctrl+D cancels the mark. The picker stays open after
deletion, including when the last saved session is removed. In `--plain`, enter
`d NUMBER` or `d ID` in `/resume`, then type `delete` to confirm. A session
open in another process cannot be deleted.

Deletion removes that session's journal, recovery copies, listing cache,
[temporary area](TEMPORARY.md) and owned complete tool outputs. Attachments no
longer referenced by another saved session are also eligible for removal; see
[attachment retention](ATTACHMENTS.md#storage-and-retention). Project files and
manual [exports](EXPORT.md) remain.
Original snapshots and backups left by a previous migration are removed too;
an unreadable or foreign original blocks deletion and remains untouched.

Deleting the current session opens a fresh conversation with the saved
model and effort defaults, or the active selection if the default provider is
unavailable. Without a usable selection, configuration is required before
deleting the current session; other saved sessions can still be deleted.
Its context and sent prompt history are cleared; unsent composer text, queued
messages and paused drafts are kept. The new
session is saved when a request is sent or inherited input contains
attachments. If deletion fails partway through, some owned files may already
be gone. Read the error and retry from `/resume`; Jecode removes the last valid
conversation record last. A foreign or unreadable journal blocks deletion and
remains untouched.

Saved sessions have no automatic retention period. Temporary files and their
explicit cleanup are described in [TEMPORARY.md](TEMPORARY.md); exports are
separate snapshots and cannot be resumed.
