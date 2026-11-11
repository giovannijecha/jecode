# Terminal interface

Running `jecode` in an interactive terminal opens the fullscreen interface
(TUI). It shows one conversation above a composer where you type, and keeps
working while you read, type or open menus. Every key is listed in
[Commands and keys](../reference/COMMANDS.md).

## The screen

From top to bottom:

- **The conversation.** Your messages, the agent's answers and a card for each
  tool call. It scrolls; the header line `>_ jecode <version>` at its top scrolls
  away with it.
- **The status row.** What the agent is doing now, or how the last turn ended.
- **The composer.** Your draft, between two horizontal borders. The lower border
  carries the conversation's [permission level](PERMISSIONS.md) on the
  right and, while a draft exists, `Ctrl+G commands` on the left.
- **The footer.** Your working directory on the left; the model, its effort and
  a context gauge on the right. Menus and panels open in place of the composer
  and the footer.

### Startup screen

While the conversation is empty, Jecode shows a startup screen: the `>_ jecode`
mark, then the version, how many [personal skills](SKILLS.md) are
available and whether the folder has an
[`AGENTS.md`](PROJECT_INSTRUCTIONS.md). If this folder has saved
conversations, up to three recent ones are listed under **Recent in this
folder**, with a `/resume` shortcut for all of them. A configured model with no
local conversations shows **No saved conversations in this folder**. With saved
conversations and no model, the recent list stays visible alongside the **No model**
warning. With neither a model nor saved conversations, **Start here** suggests
`/model`, `/permissions` and `/help`.
A row of keys follows:
`/ commands`, `Alt+V image` (`Ctrl+V` on macOS) and `Ctrl+Q quit`. On small
terminals the screen drops the keys, then shrinks the mark, then falls back to a
one-line header with the same recent conversations or suggestions. Your first
message replaces it.

## Send messages

Type and press Enter. Ctrl+J inserts a newline; Shift+Enter and Alt+Enter also
work when your terminal reports them separately. Pasted text keeps its line
breaks. The composer grows to show up to a third of the screen, five to ten
rows, and scrolls around the caret; an empty composer shows `Ask anything…`.

Ctrl+P and Ctrl+N browse your last 50 sent messages (commands are excluded),
and Esc returns to the draft you were writing. To attach files or a clipboard
image, see [Attachments](ATTACHMENTS.md).

Type `/` to open the command list; your draft filters it. Up/Down select, Tab
completes the name and Enter runs it. Press Ctrl+G instead to open the same
commands in a **Commands** sheet with its own **Filter** field, leaving your
draft untouched: it waits above the sheet as `draft kept`. Use `/` when the
command takes arguments. `/help` (or F1) lists commands and common shortcuts;
[Commands and keys](../reference/COMMANDS.md) has the full reference.

## While the agent works

The status row shows a blinking `>_`, the current activity (the running tool's
description, or a phase such as `Thinking`), the elapsed time and the turn's
token counts: `↑` input tokens sent, cached input included, and `↓` tokens
generated. The counts appear while no tool progress is shown. The composer stays editable.

- **Add a message.** Press Enter to queue it. It appears above the status row
  marked `queued`, and the agent receives it before its next model request. Only
  one message can wait at a time.
- **Take it back.** Press Up in an empty composer to edit the pending message,
  unless the agent has already accepted it.
- **Stop.** Press Esc or Ctrl+C. The status row shows `Stopped`; a new message
  continues from there.
- **Use menus.** `/settings`, `/model`, `/effort`, `/routing`, `/skills` and
  `/help` open immediately; what they change for this conversation applies from
  the next turn. `/copy`, `/process`, `/agents`, `/ask`, `/todo`, `/permissions`,
  `/mcp` and `/lsp` also run immediately. Commands that start work or replace the
  conversation, such as `/init`, `/new`, `/resume`, `/export`, `/tmp`
  and `/exit`, wait until the turn completes and any open menu
  closes; if the turn stops or fails, they return as a draft and do not run.
  With slash completion open, Enter accepts the selected full command before
  queueing it. Commands with arguments keep their typed text. Ctrl+G lists
  waiting commands too, in dim text marked `after this turn`; they
  cannot be selected there and are never queued.

When the turn ends, the status row keeps `Done`, `Stopped` or `Failed` with its
duration and facts until you send again; a failed turn also leaves its error as
the last conversation row. If a turn stops or fails before producing any answer
or tool card, your message and any queued message that had joined the turn return
to the composer as one draft, with their attachments, and leave the conversation.
Once the turn has output, a message that joined it stays in the conversation. A
queued message that had not joined returns to the composer when the composer is
free; otherwise it waits above the status row as a `paused` draft, which is never
sent automatically. Up in an empty composer takes it back.

Permission requests and questions from the agent take over the composer area
until you answer; see [Permissions](PERMISSIONS.md#approval-requests)
and [Questions and plans](QUESTIONS_AND_PLANS.md).

## Notices

Short messages appear in the status row and never become part of the
conversation:

- **Results** (`✓`), such as a copied answer or a changed setting (`old → new`),
  clear after five seconds or when you type.
- **Warnings** (`▲`) stay until your next key; **errors** (`✕`) until your next
  message. When there is a fix, they name it as a key, such as
  `/settings update key` for a rejected API key.
- **Retries** show `Retrying in 4s` with the attempt number and reason; see
  [Long-running work](LONG_WORK.md#connection-problems).

## The footer

The right side shows the model and effort, then an eight-cell context gauge. The
gauge fills toward the **Auto-compact at** threshold, so a full gauge means
compaction is due, and turns amber within three points of it. It shows the last
provider reading; a leading `~`, as in `~52%`, marks an estimate of the next
request instead, shown while it differs from that reading by at least one point
of the window. `<1%` means a little context is used; `?` means there is no
reliable figure yet. See
[Long-running work](LONG_WORK.md#context-and-compaction).

While collaborators are working or waiting for review, an `AGENTS` badge with
counts replaces the directory; see
[Collaborating agents](COLLABORATION.md#watch-in-the-tui). While processes
keep running after their calls returned, a `PROC` badge with the count and the
`/process` command does the same; with collaborators too, the `AGENTS` badge stays
and counts them among its facts. On narrow
terminals the footer drops the provider name, then shows the project name
instead of the path, then drops the effort and the rest.

## Menus and panels

Menus and panels, called sheets, take the composer's place; the footer gives
way too. From the top a sheet has the status row, a titled rule, an optional
field, the options or text, and a row of keys. A third of the screen, between
six and twelve rows, holds the options. Your draft is never touched: it waits
above the sheet as `draft kept`, next to a queued message if there is one. Only
`/` completion keeps the composer.

Up/Down and Enter choose; menus with up to nine options also accept the number
keys `1`–`9`, and longer ones filter as you type. The current value is marked
with the word `current`, separately from the highlighted row. Esc goes back one
level and closes the menu; when you have marked something for deletion, Esc
first cancels the mark. A text field, such as an API key, sits on its own row
under the title, with its label before the value, and secrets show as dots.

From a sheet that Ctrl+G opened, a command that opens another sheet shows it
over your draft; Esc returns to the **Commands** sheet one level at a time, and
a choice returns you to the composer. `/new` and `/resume` carry your draft into
the conversation they open; a resumed conversation's own saved draft joins below
it.

## Processes

`/process` lists the conversation's running processes, then its recent ones, the
latest first, each with its state or outcome, command and time. It opens at once,
also during a turn, and Ctrl+G reaches it with a draft in the composer. Select one
and press Enter to view its saved output; Tab switches between the **Output** and
**Errors** tabs, the arrows, page keys and mouse wheel scroll, Home jumps to the
start and End follows new output. Ctrl+D asks to stop a running process; Enter
confirms. When a call is waiting for the selected process, `b` moves it to the
background: the call returns, and the command keeps running without restarting.
For an interactive process, press `i` to type into it directly; Ctrl+] leaves
direct input, and Ctrl+C there sends an interrupt. Nothing you do in `/process`
enters the conversation or affects what the model sees.

`/process ID` opens one process directly, including the saved output of one that
has ended, and `/process stop ID` stops it. Esc during a turn stops the turn and any
command whose call is still in its default wait; processes already in the background
keep running.

Tool cards show a process's state when the call returned. When a process that no
call is waiting for ends, a `PROC` row reports it at that point in the
conversation. Completion creates no status-row notice: the current activity
or the turn's final statistics stay visible. An idle conversation continues from
there unless the last turn was stopped or failed, or queued messages, a menu, a
question or a permission prompt come first. See
[Processes](TOOLS.md#processes).

## Reading and scrolling

Page Up and Page Down move by about a page; the mouse wheel moves three rows.
Alt+Home jumps to the beginning and Alt+End returns to the latest output. Up and
Down edit the draft, not the conversation.

Scrolling up stops the view from following new output, so answers and tool
results can arrive without moving the page you are reading. The row above the
status row then reads `↓ N rows below`, adding `· new output` when something
arrived. Scroll to the end, press Alt+End or send a message to follow again.
Esc never moves the view.

Mouse clicks and text selection are not supported; use
[`/copy`](COPY_AND_EXPORT.md#copy) to copy answers. Your terminal's own
scrolling shortcuts take precedence over Jecode's.

When you resize the terminal, the composer and panels adapt at once and the
conversation reflows after a short pause, without interrupting typing or the
agent. Until each block is wrapped again it stays on the page, shortened to the
new width; rewrapping starts from the end of the conversation. If you were
reading older content, the view stays anchored to the same text.

Other layout rebuilds, such as a tool card changing or the `Ctrl+O` key moving,
keep the page live as well: blocks not yet drawn again stay as they were, while
streaming text, tool cards and scrolling update every frame. Finished items
still waiting for a block render with the live tail.

Layout work adds no activity row: the status row continues to describe the
agent's work. Long completed responses are laid out incrementally alongside
new streaming output. Below two rows or columns, drawing pauses until the
terminal grows again.

## Quit

Ctrl+Q quits at once when nothing is running. While a turn, a background process
or a collaborator is running, it asks first, under **Quit · work is running**,
listing what will be stopped. Enter or a second Ctrl+Q stops the work and quits;
Esc or Ctrl+C cancels. `/exit` quits once the current turn completes; if the turn
stops or fails, it returns as a draft instead. Ctrl+C never quits.

On exit, Jecode restores your terminal's screen, cursor and modes, and prints a
summary with the command to resume:

```text
>_ jecode closed · 3 prompts · 12 tools · 4m 12s
   jecode resume 1793721600123-4812-0
```

The resume line appears only after a successful save of a conversation with at
least one message; if saving failed, the summary says `autosave failed`. The
conversation is never dumped into your terminal's scrollback; it stays in the
[saved conversation](SESSIONS.md).

## Terminal compatibility

- The TUI needs a VT-compatible terminal. If it cannot start, the error suggests
  `jecode --plain`.
- Colors are 24-bit, or the 256-color palette on a Linux or macOS terminal that does
  not declare 24-bit support; see [Theme](../development/TUI_THEME.md). `NO_COLOR` is ignored. Jecode never paints a
  background: it keeps your terminal's own. The `appearance.theme` setting, or
  `/settings` › **Theme**, chooses inks for dark or light terminals; `auto`
  follows the terminal's reported background and assumes dark without a reply.
- Bold, italics, underline, strikethrough, emoji and wide characters depend on
  your terminal and font.
- Some shortcuts may be captured by your terminal or operating system first.
- On Unix, terminals with bracketed paste keep multiline pastes separate from
  typed keys. On Windows, an Enter pressed immediately after a paste can be taken
  as part of the paste; press Enter again to send.
