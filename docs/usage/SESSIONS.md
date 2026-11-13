# Saved conversations

Jecode saves every conversation automatically under `~/.jecode/sessions/`. Each
conversation belongs to the directory where it started: you can resume it only
from that directory, and moving the project to another path does not move its
conversations. Starting `jecode` always opens a new conversation.

Each conversation owns one folder, `sessions/<project>/<session>/`, holding its
history, complete command output, [temporary files](TEMPORARY.md) and
[collaborator](COLLABORATION.md) data; the project's
[attachments](ATTACHMENTS.md) are shared in `sessions/<project>/attachments/`.
See [Files Jecode keeps](../reference/FILES.md). The folder is
created when the conversation first saves or uses one of those areas; only a
folder with a saved history is a conversation you can resume.

## Resume a conversation

```text
jecode resume
jecode resume SESSION_ID
jecode resume --last
jecode --plain resume SESSION_ID
jecode resume --last "Continue with the tests"
```

`jecode resume` opens a picker of this directory's conversations, and
`jecode resume --last` opens the most recent one. With a prompt or piped stdin,
the message is sent and Jecode exits; see
[Command line](CLI.md#continue-a-conversation). `jecode sessions list` lists
the IDs without opening anything, and `jecode sessions show SESSION_ID` prints
one conversation's details and messages. In the TUI,
`/resume` opens the same picker and `/resume ID` opens one directly. The newest
conversations come first, each with its title and age (`just now`, `5m`, `2h`,
`3d`); type to filter by title, provider, model or ID. When you quit the TUI, it
prints the command to resume what you were working on:

```text
>_ jecode closed · 3 prompts · 12 tools · 4m 12s
   jecode resume 1793721600123-4812-0
```

Resume restores the history, tool results, provider, model, effort, routing,
permission level and approvals, prompt history and any unsent input. It uses the
current credentials for that provider. It does not run anything again:
interrupted requests and tool calls are not replayed, and background processes
from the earlier run are gone (their saved output stays readable). A
conversation open in another Jecode process cannot be resumed or deleted until
that process exits.

Explicit CLI options replace the corresponding saved choices. If the old
provider is no longer configured, an explicit `--provider` and a usable model
can restore the portable history through the selected provider; defaults are
unchanged and provider-specific history retains its compatibility boundary.

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
another directory are reported and left unchanged. The current journal format
is version 4, with version 2 session state. Experimental versions do not migrate
older saved formats; those conversations cannot be resumed by this build.

Earlier versions kept each conversation's files side by side in
`sessions/<project>/` (`SESSION_ID.jsonl` and its sidecars, with `outputs/`,
`collaboration/` and a separate `~/.jecode/tmp/` tree). This build reports those
conversations in `/resume` and at startup as using an earlier storage layout,
and refuses to resume or delete them. It does not move, convert or remove any of
their files; their attachments stay in the shared pool while those histories
refer to them.

## Delete a conversation

In the TUI's `/resume` picker, select a conversation and press Ctrl+D to mark it,
then Enter to confirm; Esc or Ctrl+C cancels. The picker stays open. In
`--plain`, type `d NUMBER` or `d ID` in `/resume`, then type `delete` to
confirm. From a terminal, `jecode sessions delete SESSION_ID` asks for the same
confirmation; add `--yes` when there is no terminal.

Deleting removes the conversation's saved history, recovery copies, complete
command output and [temporary files](TEMPORARY.md). Attachments are removed when
no other saved conversation uses them. Project files and
[exports](COPY_AND_EXPORT.md#export) stay. If the conversation had
[collaborators](COLLABORATION.md#stop-close-and-recover), their workspaces,
histories and records are kept in the conversation's `collaboration/` folder and
the result says where; the deleted conversation no longer appears in `/resume`.

The files Jecode removes are checked before deletion, and the history goes
last. A link or junction in their storage paths stops the deletion. Entries of
the conversation's folder that Jecode does not recognize are left in place and
named in the result. The empty `session.lock` and its folder stay, because
another Jecode process may hold or wait on that lock; they are not a
conversation.

Deleting the current conversation opens a new one with your defaults and keeps
your unsent input. If deletion fails partway, some files may already be gone;
read the error and retry from `/resume`.

Conversations have no automatic expiry.

Jecode cannot delete unsupported saved formats or the earlier storage layout
through the picker: deletion needs a valid document to check ownership of
associated files. Remove those files manually when you no longer need them;
Jecode leaves them unchanged.
