# Composer and conversation controls

The interactive TUI keeps one conversation above an editable composer. Enter
sends a draft when idle. During a turn, it accepts the draft for the active
turn before the next model request; commands that cannot run during a turn wait
until it ends. Up to eight messages can be pending. The draft clears only after
Jecode accepts it.

Use `Ctrl+J` for a newline. `Shift+Enter` and `Alt+Enter` also work when the
terminal reports them as distinct keys. Pasted text keeps its line breaks.
The caret follows long drafts through at most five visible rows; the top rule
shows a range when some text is hidden. Attachment labels are edited as one
unit. See [Attachments](../usage/ATTACHMENTS.md) for file drops and clipboard
images.

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
| `Ctrl+O` | Expand or collapse every tool in the latest finished execution turn. |
| `Ctrl+Q` | Cancel active work, save input, and quit. |

Prompt history preserves the unsent draft and caret. `Esc` leaves history and
returns to that draft. `Ctrl+O` is ignored during generation and becomes
available when the worker ends, including after a stop or error. It opens all
tools from that turn; pressing it again restores the normal previews. Both
actions preserve the reading position, including following the tail. A finished
turn without tools has no toggle, and earlier turns are never substituted. Typing
an unsent draft keeps details open. Accepting a new user prompt collapses the
old details until the new turn finishes.
Expansion is temporary. Resumed sessions start compact and retain execution-turn
boundaries, including messages accepted during generation. The shortcut is
listed in `/help`; it adds no persistent composer hint.

## Activity and queue

While a turn runs, the composer's upper rule carries a sweeping scanner and the
turn's elapsed time, which reads in hours past 60 minutes, then any draft
context, and `esc` at the right while Esc stops the turn. The composer stays
editable. Pending messages appear above it as `queued`; Jecode accepts ordinary messages before the active turn's next model
request. Most local commands wait for the turn to finish; `/copy`, `/drafts`,
and `/terminal` open immediately. If the turn stops or fails,
messages that have not joined it become `paused` and require review.

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

## Commands and selectors

Type `/` to open the commands panel; the draft is its search field. A bare
`/` lists every command under its group (Conversation, Model, Workspace,
Jecode); further typing filters a flat list. `Up`/`Down` selects a
suggestion, `Tab` completes its name, and `Enter` runs it when idle. After a
space the panel shows the typed command's description. `Esc` closes the panel
without clearing the draft. `/help` opens Commands and Keys sections, switched
with `Left`/`Right`; `F1` opens the same help while idle.

Menus share one layout: a header with the title, context and a right-aligned
scope or count; a divider; an optional search field; the list; a two-row
detail strip for the selected item's description; and a row of key hints.
Menus use `Up`/`Down` and `Enter`. Lists with at most eight options also
accept number keys; larger lists provide a search field. `Esc` closes a menu,
goes back from a settings value, or first cancels a pending delete/discard
confirmation. Current values remain marked independently of the highlighted
row. On short terminals the detail strip leaves first, then spacing, then
column headers; the selected row, search field and key hints stay.

- `/model [ID]` chooses a model and effort for this conversation;
  `/effort [NAME]` changes effort directly.
- `/routing` chooses automatic or a specific endpoint for this conversation.
  See [Configuration](../usage/CONFIGURATION.md#endpoint-routing).
- `/settings` edits saved defaults, automatic compaction threshold, routing,
  and the OpenRouter key. See [Configuration](../usage/CONFIGURATION.md).
- `/resume [ID]` opens saved conversations in this folder. In the list,
  `Ctrl+D` marks one for deletion; `Enter` confirms and `Esc` cancels. See
  [Sessions](../usage/SESSIONS.md).
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
  queue and pause rules as other drafts. `Ctrl+Q` quits immediately, stopping any
  active work. A stopped or failed turn pauses an already queued `/exit` too.

An unknown command reports an error with a spelling suggestion when one is
available. `/help` also shows the commands omitted from this quick reference.

## Processes

In `/terminal`, select a process with `Up`/`Down` and open it with `Enter`.
The output view has Output and Errors tabs: `Tab` switches streams,
the arrows, page keys, and wheel browse, `Home` jumps to the start and `End`
follows the tail. Output wraps at word boundaries. `Ctrl+D`
opens a stop confirmation for the selected process; `Enter` confirms and
`Esc` or `Ctrl+C` cancels. `/terminal stop ID` requests the same stop directly.
The composer and pending drafts remain available after the panel closes.
See [Tools](../usage/TOOLS.md) for process behavior.

## Reading, closing, and feedback

`Page Up`/`Page Down` and the mouse wheel browse the conversation. Scrolling
up holds the reading position while new output arrives. `Alt+Home` goes to
the start; `Alt+End` follows the latest output. `Esc` returns to the bottom
while reading, but closes an open panel, draft edit, or history first. At the
bottom, `Esc` stops active work. `Ctrl+C` also stops
work, closes a panel, or clears an idle draft; it never exits. See
[Scrolling and resize](RESIZE.md) for reflow and terminal restoration.

Notices appear above the composer, by lifetime. Results and setting changes
are muted text without an icon, clearing after five seconds or when you start
typing. A changed setting's new value stays strong.
Warnings and errors are banners that stay until your next key, naming the
next step when there is one, such as `Alt+↑ review` for paused drafts or
`/settings update key` for a rejected key; a lower-severity notice never
replaces them. Progress such as compaction, attachments or a retry sits in
the composer rule beside the scanner, with a countdown to the next attempt.
Copy feedback has its own slot. Notices do not become conversation messages.
The rule also names the turn's phase, including `Waiting for your edit` when
a draft edit holds the turn and `Stopping` after cancellation. Tool activity
uses `Working`, which also covers the interval after a tool finishes.
Maintenance and retry feedback take priority over normal phases. A retry temporarily
overlays compaction; its original progress and clock return when the retry ends.

The footer shows the working directory relative to your home folder, a
context meter with the projected share, the model and effort. The meter fills
toward the configured automatic compaction threshold, so a full meter means
compaction is due; it turns gold within three points of it, starting at 1%
for thresholds of 3% or less. An empty meter
with `?` means there is no reliable capacity yet. Narrow terminals show the
project name and the model without its provider, then drop them before the
meter. Unsent draft text is excluded from the projection. See [Configuration](../usage/CONFIGURATION.md) for model and
compaction settings.

Terminal shortcuts can be intercepted by the host. On Unix, a terminal with
bracketed paste support distinguishes multiline paste from key presses. On
Windows, a very fast Enter following a pasted burst may be treated as a pasted
newline; press Enter again to send.
