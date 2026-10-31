# Terminal interface

Running `jecode` in an interactive terminal opens the fullscreen interface
(TUI). It shows one conversation above a composer where you type, and keeps
working while you read, type or open menus. Every key is listed in
[Commands and keys](COMMANDS.md).

## The screen

From top to bottom:

- **The conversation.** Your messages, the agent's answers and a card for each
  tool call. It scrolls; the header line `>_ jecode <version>` at its top scrolls
  away with it.
- **The status row.** What the agent is doing now, or how the last turn ended.
- **The composer.** Your draft, between two horizontal borders. The lower border
  carries the conversation's [permission level](../usage/PERMISSIONS.md).
- **The footer.** Your working directory on the left; the model, its effort and
  a context gauge on the right. Menus and panels open in its place.

### Startup screen

While the conversation is empty, Jecode shows a startup screen: the `>_ jecode`
mark and version, a tip about `/permissions`, how many
[personal skills](../usage/SKILLS.md) are available, whether the folder has an
[`AGENTS.md`](../usage/PROJECT_INSTRUCTIONS.md), and a row of key shortcuts. If
this folder has saved conversations, up to three recent ones are listed under
**Recent in this folder**, with a `/resume` shortcut for all of them. On small
terminals the screen drops the tip and shortcuts and falls back to a compact
version. Your first message replaces it.

## Send messages

Type and press Enter. Ctrl+J inserts a newline; Shift+Enter and Alt+Enter also
work when your terminal reports them separately. Pasted text keeps its line
breaks. The composer grows to show up to a third of the screen and scrolls
around the caret; an empty composer shows `Ask anything…`.

Ctrl+P and Ctrl+N browse your last 50 sent messages (commands are excluded),
and Esc returns to the draft you were writing. To attach files or a clipboard
image, see [Attachments](../usage/ATTACHMENTS.md).

Type `/` to open the command list; your draft filters it. Up/Down select, Tab
completes the name and Enter runs it. `/help` (or F1) lists commands and common
shortcuts; [Commands and keys](COMMANDS.md) has the full reference.

## While the agent works

The status row shows a blinking `>_`, the current activity (the running tool's
description, or a phase such as `Thinking`), the elapsed time and the number of
tokens generated. The composer stays editable.

- **Add a message.** Press Enter to queue it. It appears above the status row
  marked `queued`, and the agent receives it before its next model request. Only
  one message can wait at a time.
- **Take it back.** Press Up in an empty composer to edit the pending message,
  unless the agent has already accepted it.
- **Stop.** Press Esc or Ctrl+C. The status row shows `Stopped`; a new message
  continues from there.
- **Use menus.** `/settings`, `/model`, `/effort`, `/routing`, `/skills` and
  `/help` open immediately; what they change for this conversation applies from
  the next turn. `/copy`, `/terminal`, `/agents`, `/ask`, `/todo` and
  `/permissions` also run immediately. Commands that start work or replace the
  conversation, such as `/init`, `/new`, `/resume`, `/export`, `/tmp`,
  `/checkpoints` and `/exit`, wait for the turn to end.

When the turn ends, the status row keeps `Done`, `Stopped` or `Failed` with its
duration and facts until you send again; a failed turn also leaves its error as
the last conversation row. If a turn stops or fails before producing any answer
or tool card, your message returns to the composer with its attachments and is
removed from the conversation. A queued message that had not joined the turn
returns to the composer too; one that had joined stays as a `paused` draft that
is never sent automatically.

Permission requests and questions from the agent take over the composer area
until you answer; see [Permissions](../usage/PERMISSIONS.md#approval-requests)
and [Questions and plans](../usage/QUESTIONS_AND_PLANS.md).

## Notices

Short messages appear in the status row and never become part of the
conversation:

- **Results** (`✓`), such as a copied answer or a changed setting (`old → new`),
  clear after five seconds or when you type.
- **Warnings** (`▲`) stay until your next key; **errors** (`✕`) until your next
  message. When there is a fix, they name it as a key, such as
  `/settings update key` for a rejected API key.
- **Retries** show `Retrying in 4s` with the attempt number and reason; see
  [Long-running work](../usage/LONG_WORK.md#connection-problems).

## The footer

The right side shows the model and effort, then an eight-cell context gauge. The
gauge fills toward the **Auto-compact at** threshold, so a full gauge means
compaction is due, and turns amber within three points of it. `<1%` means a
little context is used; `?` means there is no reliable figure yet. See
[Long-running work](../usage/LONG_WORK.md#context-and-compaction).

While collaborators are working or waiting for review, an `AGENTS` badge with
counts replaces the directory; see
[Collaborating agents](../usage/COLLABORATION.md#watch-in-the-tui). On narrow
terminals the footer drops the provider name, then shows the project name
instead of the path, then drops the effort and the rest.

## Menus and panels

Menus and panels open below the composer, using up to two thirds of the screen.
Up/Down and Enter choose; menus with up to eight options also accept number
keys, and longer ones filter as you type. The current value is marked
separately from the highlighted row. Esc goes back one level and closes the
menu; when you have marked something for deletion, Esc first cancels the mark.
Text fields, such as an API key, appear in the composer area, and secrets show
as dots.

## Background processes

`/terminal` lists the conversation's background processes. Select one and press
Enter to view its output; Tab switches between the **Output** and **Errors**
tabs, the arrows, page keys and mouse wheel scroll, Home jumps to the start and
End follows new output. Ctrl+D asks to stop the selected process; Enter confirms.
For an interactive process, press `i` to type into it directly; Ctrl+] leaves
direct input, and Ctrl+C there sends an interrupt. Nothing you do in `/terminal`
enters the conversation or affects what the model sees.

`/terminal ID` opens one process directly, and `/terminal stop ID` stops it.
Tool cards show a process's state when the call returned; `/terminal` shows its
current state.

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
[`/copy`](../usage/COPY_AND_EXPORT.md#copy) to copy answers. Your terminal's own
scrolling shortcuts take precedence over Jecode's.

When you resize the terminal, the composer and panels adapt at once and the
conversation reflows after a short pause, without interrupting typing or the
agent. If you were reading older content, the view stays anchored to the same
text. The row above the status row may show `Reflowing conversation…` meanwhile.
Below two rows or columns, drawing pauses until the terminal grows again.

## Quit

Ctrl+Q quits at once when nothing is running. While a turn, a background process
or a collaborator is running, it asks first, under **Quit · work is running**,
listing what will be stopped. Enter or a second Ctrl+Q stops the work and quits;
Esc or Ctrl+C cancels. `/exit` quits after the current turn. Ctrl+C never quits.

On exit, Jecode restores your terminal's screen, cursor and modes, and prints a
summary with the command to resume:

```text
>_ jecode closed · 3 prompts · 12 tools · 4m 12s
   jecode resume 7f3a9c2e
```

The resume line appears only after a successful save of a conversation with at
least one message; if saving failed, the summary says `autosave failed`. The
conversation is never dumped into your terminal's scrollback; it stays in the
[saved conversation](../usage/SESSIONS.md).

## Terminal compatibility

- The TUI needs a VT-compatible terminal. If it cannot start, the error suggests
  `jecode --plain`.
- Colors are 24-bit and `NO_COLOR` is ignored. Jecode keeps dark terminal
  backgrounds; on a terminal it detects as light, it paints its own dark
  background without changing your terminal's palette.
- Bold, italics, underline, strikethrough, emoji and wide characters depend on
  your terminal and font.
- Some shortcuts may be captured by your terminal or operating system first.
- On Unix, terminals with bracketed paste keep multiline pastes separate from
  typed keys. On Windows, an Enter pressed immediately after a paste can be taken
  as part of the paste; press Enter again to send.
