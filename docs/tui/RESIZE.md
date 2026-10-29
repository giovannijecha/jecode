# Scrolling and resize

The interactive TUI uses the terminal's alternate screen and keeps the
terminal's own background outside its surfaces. The conversation, opened by
the header, scrolls above a dock anchored at the bottom. `--plain`, single
prompts and piped input keep line-oriented output. Use a VT-capable terminal;
if Jecode cannot initialize one, its error points to `jecode --plain`.

## Reading and following

The conversation initially follows new output. `Page Up`/`Page Down` moves by
nearly one page, with a row of overlap; the mouse wheel moves three rows per
notch. `Alt+Home` goes to the beginning and `Alt+End` follows the latest
output. `Esc` never moves the view: it closes panels and menus, or stops
active work.

Scrolling up pauses following. New response text and tool results can arrive
without moving the page you are reading. While the view is away from the
tail, the row above the dock reads `↓ N rows below`, adding `· new output`
once output arrived after you scrolled away; a resize reflow alone does not
count. Its keycap names the way back, `Alt+End back to bottom`, and drops
first when narrow. Scroll down to the end, press `Alt+End`, or send a new
prompt to resume following.

`Up`/`Down` edit the draft instead of scrolling the conversation. An open
selector, information panel, or process view receives its own navigation and
wheel input first. Mouse clicks are not supported and Jecode draws no
selection; use [`/copy`](../usage/COPY.md) to copy completed original text.

## Resize lifecycle

When the terminal changes size, the composer and open panel adapt to the
latest dimensions while the conversation reflows after a short pause in
resize events. Typing and generation continue during this work. Width changes
rebuild wrapped text and tables from the original response; height changes
adjust how many rows are visible. The row above the dock may read
`Reflowing conversation…` meanwhile.

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
`Ctrl+Q` saves the draft and exits; while a turn, background process or
collaborator runs,
it first asks to stop that work (see
[Composer and controls](COMPOSER.md#reading-quitting-and-feedback)). On normal
exit, Jecode resets attributes, restores the caller's screen, cursor, and
terminal modes, then prints a summary in the terminal's own colours:

```text
>_ jecode closed · 3 prompts · 12 tools · 4m 12s
   jecode resume 7f3a9c2e
```

`>_` is ANSI magenta, `jecode` bold and the facts dim; the resume command uses
the default foreground and appears only after a successful save of a
conversation with at least one prompt. When saving failed, `· autosave failed`
joins the facts and no resume command is printed.

Conversation messages and tool results remain in the saved session and
[export](../usage/EXPORT.md); the fullscreen display is not dumped into the
caller's scrollback.
