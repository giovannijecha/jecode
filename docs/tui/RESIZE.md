# Scrolling and resize

The interactive TUI uses the terminal's alternate screen. The conversation
scrolls above a composer anchored at the bottom. `--plain`, single prompts,
and piped input keep line-oriented output. Use a VT-capable terminal; if
Jecode cannot initialize one, its error points to `jecode --plain`.

## Reading and following

The conversation initially follows new output. `Page Up`/`Page Down` moves by
nearly one page, with a row of overlap; the mouse wheel moves three rows per
notch. `Alt+Home` goes to the beginning and `Alt+End` follows the latest
output. `Esc` returns to the bottom while reading, after any open panel,
draft edit, history, or tool inspection has been closed.

Scrolling up pauses following. New response text and tool results can arrive
without moving the page you are reading. A Back to bottom badge appears while
the viewport is away from the tail. Scroll down to the end, press `Alt+End`,
or use `Esc` in the ordinary reading view to resume following.

`Up`/`Down` edit the draft instead of scrolling the conversation. An open
selector, information panel, or process view receives its own navigation and
wheel input first. Mouse clicks and selection are not supported by Jecode;
use [`/copy`](../usage/COPY.md) to copy completed original text.

## Resize lifecycle

When the terminal changes size, the composer and open panel adapt to the
latest dimensions while the conversation reflows after a short pause in
resize events. Typing and generation continue during this work. Width changes
rebuild wrapped text and tables from the original response; height changes
adjust how many rows are visible. A temporary reflow hint may appear.

If you are reading older content, Jecode anchors the page to the source text
near its top rather than jumping to the latest output. Repeated width changes
keep that reading point, including within wrapped paragraphs and tables.
Growing the terminal can leave space below an anchored page until you scroll.
If you were following the tail, you continue following it after resize.

The unsent draft, caret, pending messages, selector query and selection, and
active turn survive resize. On a very small terminal, supporting rows yield
space to input and selection. Painting pauses if the terminal has fewer than
two rows or columns; normal size restores the current view.

Only the active conversation is shown. `/new`, `/resume`, and deleting the
current session reset the viewport for the resulting conversation. Saved
conversations remain available through `/resume`; see
[Sessions](../usage/SESSIONS.md).

## Entering and leaving

Entering the TUI leaves the caller's main screen and scrollback in place.
`Ctrl+Q` cancels active work, saves the draft, and exits. On normal exit,
Jecode restores the caller's screen, cursor, and terminal modes before
printing a short prompt/tool count. If the conversation was saved, it also
prints a `jecode resume ID` command. An empty unsaved conversation has no
resume command; a save failure is reported instead.

Conversation messages and tool results remain in the saved session and
[export](../usage/EXPORT.md); the fullscreen display is not dumped into the
caller's scrollback.
