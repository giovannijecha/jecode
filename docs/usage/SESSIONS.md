# Saved conversations

Jecode saves every conversation automatically under `~/.jecode/sessions/`. Each
conversation belongs to the directory where it started: you can resume it only
from that directory, and moving the project to another path does not move its
conversations. Starting `jecode` always opens a new conversation.

## Resume a conversation

```text
jecode resume
jecode resume SESSION_ID
jecode --plain resume SESSION_ID
```

`jecode resume` opens a picker of this directory's conversations. In the TUI,
`/resume` opens the same picker and `/resume ID` opens one directly. The newest
conversations come first, each with its title and age (`just now`, `5m`, `2h`,
`3d`); type to filter by title, provider, model or ID. When you quit the TUI, it
prints the command to resume what you were working on:

```text
>_ jecode closed · 3 prompts · 12 tools · 4m 12s
   jecode resume 7f3a9c2e
```

Resume restores the history, tool results, provider, model, effort, routing,
permission level and approvals, prompt history and any unsent input. It uses the
current credentials for that provider. It does not run anything again:
interrupted requests and tool calls are not replayed, and background processes
from the earlier run are gone (their saved output stays readable). A
conversation open in another Jecode process cannot be resumed or deleted until
that process exits.

`/new` starts a separate conversation with your saved defaults and keeps any
unsent text in the composer. `/model` switches the model inside the same
conversation instead.

## Titles

After your first request is saved, Jecode asks the conversation's model, at low
effort, for a short title in the language of your request. Until it arrives, or
if it fails, the title is the first request itself. A single prompt can exit
before the title request finishes.

## When a conversation is saved

A conversation is saved when you send its first request, before any provider
work starts, or when you add an attachment to the composer. Opening Jecode,
typing a draft or running local commands alone saves nothing, so a draft typed
before the first request is lost when you quit.

From then on, Jecode saves around every model request and tool call, and keeps
your unsent draft, the pending message and your prompt history. On resume, a
queued message becomes a paused draft: press Up in an empty composer to take it
back, then Enter to send it. Nothing pending is sent automatically.

If you send a task before any provider is configured, it returns to the
composer. A draft with attachments can still be saved; it appears as **Not
selected** in the picker and uses whichever model is configured when resumed.

## After an interruption

If Jecode stops unexpectedly, resuming shows exactly what is known:

- Tool calls whose results were saved keep them, with their file diffs.
- A call that had started but whose result was not saved is marked **unknown**.
  Check the project files and anything else it could have affected before
  retrying.
- Calls that had not started are marked **not executed**.
- Partially streamed text is kept as incomplete.

A background process reported by `process_id` may still be running; check
before assuming it finished. A forced exit can lose the latest unsaved draft
edit or streamed text.

If the initial save fails, Jecode does not contact the provider or run tools.
Fix the storage problem and send the work again. A damaged final save is
recovered from the previous complete one, with the incomplete bytes kept
separately. Damaged earlier records, unsupported formats and conversations from
another directory are reported and left unchanged. Conversations saved by
versions before the current storage format (version 3) cannot be opened.

## Delete a conversation

In the TUI's `/resume` picker, select a conversation and press Ctrl+D to mark it,
then Enter to confirm; Esc or Ctrl+C cancels. The picker stays open. In
`--plain`, type `d NUMBER` or `d ID` in `/resume`, then type `delete` to
confirm.

Deleting removes the conversation's saved history, recovery copies, complete
command output and [temporary files](TEMPORARY.md). Attachments are removed when
no other saved conversation uses them. Project files and
[exports](COPY_AND_EXPORT.md#export) stay. If the conversation had
[collaborators](COLLABORATION.md#stop-close-and-recover), their files and
histories are kept and the result says where.

Deleting the current conversation opens a new one with your defaults and keeps
your unsent input. If deletion fails partway, some files may already be gone;
read the error and retry from `/resume`.

Conversations have no automatic expiry.
