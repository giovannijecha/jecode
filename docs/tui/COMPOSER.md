# Composer and conversation controls

The interactive TUI keeps one conversation above an editable composer. Enter
sends a draft when idle. During a turn, it accepts the draft for the active
turn before the next model request; commands that cannot run during a turn wait
until it ends. Up to eight messages can be pending. The draft clears only after
Jecode accepts it.

The composer sits between two full-width rules: `›` marks the first visible
row, the draft is `STRONG`, and an empty draft shows `Ask anything…`. Use
`Ctrl+J` for a newline. `Shift+Enter` and `Alt+Enter` also work when the
terminal reports them as distinct keys. Pasted text keeps its line breaks. The
draft shows up to a third of the screen, five to ten rows, centred on the
caret; the top rule shows the visible range when some rows are hidden, and
names the context: `Drafts 3 · Alt+↑` with pending drafts, `Editing draft 1/3`
or `Prompt history`. Attachment labels are chips edited as one unit. See
[Attachments](../usage/ATTACHMENTS.md) for file drops and clipboard images.

## Editing keys

| Key | Action |
| --- | --- |
| `Enter` | Send the draft, or accept it during a turn. |
| `Ctrl+J` | Insert a newline. |
| `Left` / `Right`, `Ctrl+Left` / `Ctrl+Right` | Move by character or word. |
| `Up` / `Down` | Move between visual rows of the draft. |
| `Home` / `End`, `Ctrl+A` / `Ctrl+E` | Move to the start or end of the logical line. |
| `Ctrl+Home` / `Ctrl+End` | Move to the start or end of the whole draft. |
| `Backspace` / `Delete` | Delete before or after the caret; remove an attachment whole. |
| `Ctrl+Backspace` / `Ctrl+Delete`, `Ctrl+W` | Delete by word when the terminal reports the shortcut. |
| `Ctrl+U` | Delete to the start of the logical line. |
| `Ctrl+P` / `Ctrl+N` | Browse up to 50 sent prompts; commands are excluded. |
| `Alt+V` | Attach the clipboard image. `Ctrl+V` remains the terminal's text paste. |
| `Alt+Up` | Open pending drafts. |
| `Ctrl+O` | Expand or collapse the latest collaboration receipt, or tools in the latest finished execution turn. |
| `Ctrl+Q` | Quit; while work runs, ask first. |

Prompt history preserves the unsent draft and caret. `Esc` leaves history and
returns to that draft. While a draft edit or the history holds the composer,
its keys (`Enter save · Esc cancel`, `Enter send · Ctrl+P/N history · Esc
back`) replace the footer.

`Ctrl+O` is ignored during generation and becomes available when the worker
ends, including after a stop or error. It opens all tools from that turn;
pressing it again restores compact single-call cards and Work chapter
summaries, with each Bash command still visible. Both actions preserve the
reading position, including following the tail. A finished turn without tools
has no toggle, and earlier turns are never substituted. Typing an unsent draft
keeps details open. Accepting a new user prompt collapses the old details until
the new turn finishes. Expansion is temporary. Resumed sessions start compact
and retain execution-turn boundaries, including messages accepted during
generation. The keycap on the turn's last hidden card, the first-run startup
row and `/help` name the shortcut.

If a collaboration receipt is newer than the last ordinary tool card, `Ctrl+O`
toggles that receipt's historical assignment snapshots. `/agents` opens a
read-only list immediately, including during generation. Enter opens Activity,
Assignment and Result tabs. Esc returns detail → list → main draft, preserving
the draft and caret. Activity follows output until scrolled; End resumes
following. Inspecting a result never marks it reviewed. See
[collaborating agents](../usage/COLLABORATION.md).

## Activity and queue

While a turn runs, the status row above the composer shows the blinking `>_`
caret, the shimmering activity (the running call's description or verb, or the
turn's phase such as `Thinking`), `Esc interrupt` while Esc stops the turn, the
elapsed time, which reads in hours past 60 minutes, and the generated tokens.
The composer rules turn faint and the composer stays editable. Pending
messages appear above the status row as `› first line…` with `queued` or
`paused` on the right; Jecode accepts ordinary messages before the active
turn's next model request. Most local commands wait for the turn to finish;
`/copy`, `/drafts`, and `/terminal` open immediately. If the turn stops or
fails, messages that have not joined it become `paused` and require review.

When the turn ends, the status row keeps `Done`, `Stopped` or `Failed` with
its time and facts until the next send. A failed turn also leaves its
diagnostic as the turn's last conversation row.

If a turn fails or is stopped with no response text or tool card left in the transcript,
its first prompt returns to the composer with its attachments. Other messages
already accepted into that unanswered turn become paused drafts; a newer
composer draft or edit is preserved there too. Returned input disappears from
the transcript and is excluded from later model requests, including after
resuming. Waiting and thinking indicators do not count as output. A streaming
preview discarded during recovery does not count either. Confirmed response
text, a retained partial response or a tool card keeps the transcript and
composer in place, so a new message such as `continue` can resume the work.

Open `/drafts` or press `Alt+Up` to review pending messages, even during a
turn. Queued drafts appear before paused ones. `Enter` edits the selected
draft; in the editor, `Enter` saves the edit and `Esc` cancels it. `Ctrl+S`
sends a paused draft explicitly, including from its editor. `Ctrl+D` marks a
draft for discard; `Enter` confirms and `Esc` cancels. Queued drafts already
set for automatic sending cannot be sent again with `Ctrl+S`.

Jecode saves the composer, pending drafts, and sent-prompt history with the
conversation. Resuming a session never sends paused drafts automatically.
See [Sessions](../usage/SESSIONS.md) for persistence and resume behavior, and
[Long work](../usage/LONG_WORK.md) for turn recovery and compaction.

## Commands and panels

Type `/` to open the commands panel; the draft is its filter. A bare `/` lists
every command under its group (Conversation, Model, Workspace, Jecode); further
typing filters a flat list. `Up`/`Down` selects a suggestion, `Tab` completes
its name, and `Enter` runs it when idle. After a space the panel shows the
typed command's description. `Esc` closes the panel without clearing the
draft. `/help` opens Commands and Keys tabs, switched with `Left`/`Right`;
`F1` opens the same help while idle.

Panels open below the composer in place of the footer and take at most two
thirds of the screen. The title and a count or scope ride the composer's
bottom rule; a typed field, such as `Filter models` or a pasted key, takes the
composer's place with the field named on its top rule, and a secret shows one
dot per character. The list shows a fact column beside each name, scrolls
around the selection, and ends with one blank row and its keys. Menus use
`Up`/`Down` and `Enter`. Lists with at most eight options also accept number
keys; larger lists filter as you type. `Esc` closes a menu, goes back from a
settings value, or first cancels a pending delete/discard confirmation, whose
question replaces the keys. Current values remain marked independently of the
highlighted row.

Key rows wrap onto more rows when narrow, after optional keys leave; scrolling
keys show only when the content overflows. On short terminals spacing leaves
first, then extra draft rows, tabs or column headers, the status row and top
rule, then the list rows farthest from the selection; the selected row, the
title rule and the key row holding `Esc` stay.

- `/backend [ID MODEL]` lists backends or switches this conversation to a
  configured backend and model.
- `/model [ID]` chooses a model and effort for this conversation;
  `/effort [NAME]` changes effort directly.
- `/routing` chooses automatic or a specific OpenRouter endpoint for this
  conversation; OpenAI does not expose endpoint routing here.
  See [Configuration](../usage/CONFIGURATION.md#endpoint-routing).
- `/settings` edits saved defaults, automatic compaction threshold, OpenRouter
  routing and key, and ChatGPT accounts. See
  [Configuration](../usage/CONFIGURATION.md).
- `/resume [ID]` opens saved conversations in this folder, each with a compact
  age such as `2h`. In the list, `Ctrl+D` marks one for deletion; `Enter`
  confirms and `Esc` cancels. See [Sessions](../usage/SESSIONS.md).
- `/copy` selects completed response text, code, or quotes. See
  [Copy](../usage/COPY.md).
- `/drafts` opens the pending drafts list.
- `/terminal [ID]` opens active processes or saved output.
- `/checkpoints [COMMAND]` lists, inspects or restores per-call checkpoints
  and shows the result without a model request. See
  [Checkpoints](../usage/CHECKPOINTS.md#local-commands-and-previous-archives).
- `/init` asks the current model to inspect the project and create or update the
  launch directory's `AGENTS.md`. See [Project instructions](../usage/PROJECT_INSTRUCTIONS.md).
- `/exit` quits after the current turn. Its hidden alias `/quit` follows the same
  queue and pause rules as other drafts. A stopped or failed turn pauses an
  already queued `/exit` too.

An unknown command reports an error with a spelling suggestion when one is
available. `/help` also shows the commands omitted from this quick reference.

## Processes

In `/terminal`, select a process with `Up`/`Down` and open it with `Enter`.
The output view has Output and Errors tabs: `Tab` switches streams,
the arrows, page keys, and wheel browse, `Home` jumps to the start and `End`
follows the tail; the tab row names the mode, `following` or `paused`. Output
wraps at word boundaries. `Ctrl+D` opens a stop confirmation for the selected
process; `Enter` confirms and `Esc` or `Ctrl+C` cancels. `Esc` returns from
the output to the list, then closes the panel. `/terminal stop ID` requests
the same stop directly. The composer and pending drafts remain available after
the panel closes. See [Tools](../usage/TOOLS.md) for process behavior.

## Reading, quitting, and feedback

`Page Up`/`Page Down` and the mouse wheel browse the conversation. Scrolling
up holds the reading position while new output arrives, and the row above the
composer counts the rows below. `Alt+Home` goes to the start; `Alt+End`
follows the latest output. `Esc` returns to the bottom while reading, but
closes an open panel, draft edit, or history first. At the bottom, `Esc` stops
active work. `Ctrl+C` also stops work, closes a panel, or clears an idle
draft; it never quits. See [Scrolling and resize](RESIZE.md) for reflow and
terminal restoration.

`Ctrl+Q` quits at once when idle. While a turn runs, a background process is
active or a collaborator is still on its assignment, it opens
`Quit · work is running`, listing the current turn, each process and each
working agent, with `Stop it and quit?` (`both` or `all` for more). A stopped
collaborator resumes with an unknown outcome. `Enter` or a
second `Ctrl+Q` stops the work and quits; `Esc` or `Ctrl+C` cancels. Quitting
saves input in an existing saved session.

Notices take the status row, by lifetime. A result shows `✓` with its subject
and detail and clears after five seconds or when you start typing; a changed
setting reads `old → new` with the new value strong. A warning (`▲`) stays
until your next key and an error (`✕`) until your next send, naming the next
step as a keycap when there is one, such as `Alt+↑ review` for paused drafts
or `/settings update key` for a rejected key. A notice gives way only to one
at least as severe. Progress such as compaction or attachments takes the
activity label beside the caret; a retry shows `Retrying in 4s` with the
attempt and reason, overlaying compaction until it ends. Copy feedback has its
own slot. Notices do not become conversation messages.

The footer shows the working directory relative to your home folder on the
left, and on the right a chip with the model id and effort, then an
eight-cell context gauge with the projected share. The header names only the
product and version, so these facts appear once on screen; `/settings` shows
the ChatGPT account state. The gauge fills toward the configured automatic
compaction threshold, so a full gauge means compaction is due; it turns gold
within three points of it, starting at 1% for thresholds of 3% or less. `<1%`
means some context is used; `?` means there is no reliable capacity yet.
Narrow terminals drop the model's provider, show the project name instead of
the path, then drop the effort, the project name and finally the chip. Unsent
draft text is excluded from the projection. See
[Configuration](../usage/CONFIGURATION.md) for model and compaction settings.

Terminal shortcuts can be intercepted by the host. On Unix, a terminal with
bracketed paste support distinguishes multiline paste from key presses. On
Windows, a very fast Enter following a pasted burst may be treated as a pasted
newline; press Enter again to send.
