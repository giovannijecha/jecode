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
unit. See [Attachments](../usage/ATTACHMENTS.md) for file drops, `/attach`,
and clipboard images.

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
tools from that turn and moves the view to its first tool; pressing it again
restores the normal previews and returns to the final answer. A finished turn
without tools has no toggle, and earlier turns are never substituted. Typing
an unsent draft keeps details open. Accepting a new user prompt collapses the
old details until the new turn finishes.
Expansion is temporary. Resumed sessions start compact and retain execution-turn
boundaries, including messages accepted during generation. Legacy sessions use
the last saved request block for the toggle. The shortcut is listed in `/help`;
it adds no persistent composer hint.

## Activity and queue

The activity line names the current phase and elapsed time. While a turn is
running, the composer stays editable. Pending messages appear above it as
`queued`; Jecode accepts ordinary messages before the active turn's next model
request. Most local commands wait for the turn to finish; `/copy`, `/drafts`,
`/attach`, and `/terminal` open immediately. If the turn stops or fails,
messages that have not joined it become `paused` and require review.

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

Type `/` to see command suggestions. `Up`/`Down` selects a suggestion,
`Tab` completes its name, and `Enter` runs it when idle. Adding a space keeps
the command text but closes suggestions. `Esc` closes the suggestion panel
without clearing the draft. `/help` lists the available commands and keys;
`F1` opens the same help while idle.

Selectors use `Up`/`Down` and `Enter`. Lists with at most eight options also
accept number keys; larger lists provide a search field. `Esc` closes a
selector, or first cancels a pending delete/discard confirmation. Current
values remain marked independently of the highlighted row.

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
- `/checkpoints [COMMAND]` inspects, restores or configures workspace history
  and shows the result without a model request. See
  [Checkpoints](../usage/CHECKPOINTS.md#commands).
- `/init` asks the current model to inspect the project and create or update the
  launch directory's `JECODE.md`. See [Project instructions](../usage/PROJECT_INSTRUCTIONS.md).
- `/exit` quits after the current turn. Its hidden alias `/quit` follows the same
  queue and pause rules as other drafts. `Ctrl+Q` quits immediately, stopping any
  active work. A stopped or failed turn pauses an already queued `/exit` too.

An unknown command reports an error with a spelling suggestion when one is
available. `/help` also shows the commands omitted from this quick reference.

## Processes

In `/terminal`, select a process with `Up`/`Down` and open it with `Enter`.
The output view has Output and Errors streams: `Tab` switches streams,
the arrows, page keys, and wheel browse, and `End` follows the tail. `Ctrl+D`
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

Notices appear near the composer: muted results, gold warnings, and red
errors. Short notices clear after a few seconds or on input; errors remain
until the next action. Copy feedback has its own slot. These notices do not
become conversation messages.

The footer shows the working directory, model, effort, and a projected context
share when capacity is known. `Context unknown` means there is no reliable
capacity yet. `Auto 80%` denotes the configured automatic compaction
threshold, not the current usage. Unsent draft text is excluded from the
projection. See [Configuration](../usage/CONFIGURATION.md) for model and
compaction settings.

Terminal shortcuts can be intercepted by the host. On Unix, a terminal with
bracketed paste support distinguishes multiline paste from key presses. On
Windows, a very fast Enter following a pasted burst may be treated as a pasted
newline; press Enter again to send.
