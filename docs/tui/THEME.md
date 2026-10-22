# TUI theme

Jecode's interactive TUI is designed for a dark terminal. It inherits the
terminal's ordinary text and background, then applies semantic foreground
roles and component surfaces. It emits true-color VT styling; the terminal
chooses the font and controls how bold, italic, underline, and strikethrough
appear. The palette and style aliases live in
[`theme.rs`](../../src/tui/theme.rs).

## Colour roles

| Role | Colour | Current use |
| --- | --- | --- |
| `BODY` | Inherited | Assistant prose, draft text, tool output. |
| `STRONG` | `#E2E8F0` | User text, panel titles, menu names. |
| `EMPHASIS` | `#DCE4F0` | Tool titles, activity labels, Markdown headings and strong text. |
| `CODE_TEXT` | `#C9D1D9` | Unhighlighted fenced code. |
| `MUTED` | `#9AA4B2` | Descriptions, footer, quotes, information and neutral notices. |
| `TOOL_DIM` | `#879AAA` | Expanded tool elapsed time and activity time. |
| `ACCENT` | `#7AA2F7` | Jecode mark, composer rules, focus, matches, links and inline code. |
| `TOOL_ACCENT` | `#99B8D4` | Running and background tool markers. |
| `KEYWORD` | `#BB9AF7` | Code keywords. |
| `GOOD` | `#9ECE6A` | Successful tools, added lines and code strings. |
| `BAD` | `#EF8B8B` | Failed tools, destructive choices and removed lines. |
| `WARNING` | `#E0AF68` | Warnings, stopped tools and code numbers. |

Weight and slant are applied by the component: tool names and Markdown
headings are bold, while notices are italic. Links use the accent with
underline. The palette does not replace the terminal's global colors.

## Surfaces

| Surface | Colour | Current use |
| --- | --- | --- |
| Terminal | Inherited | Conversation and notices. |
| `USER_SURFACE` | `#282E39` | Sent requests and command, selector, information and process panels. |
| `SELECTED_SURFACE` | `#353D4B` | Selected menu/process rows and Back to bottom badge. |
| `CODE_SURFACE` | `#20252C` | Fenced code. |
| `DIFF_ADDED_SURFACE` | `#1E2E23` | Added file lines, including new files. |
| `DIFF_REMOVED_SURFACE` | `#312026` | Removed file lines. |
| `CURSOR` | `#1A1B26` on `#C0CAF5` | Software caret, including in shaded panels. |

## Conversation and tool cards

The conversation has one neutral row between visible blocks. Sent user
messages have a two-cell indent and shaded padding; assistant Markdown
preserves source lines and blank rows. Fenced code spans the full row width.
See [Markdown responses](MARKDOWN.md) for the display subset.

Every call uses `● Tool(target)` and an indented `⎿` result. The display
names are Read, Skill (a `read` of a skill's `SKILL.md`), Update (the `edit`
tool), Write, Bash, Process and Checkpoint. A successful skill load uses the same green
marker; a failed read keeps the error state. Titles
wrap with a two-cell indent; result continuations and output use four cells.
Compact titles and individual output lines wrap through at most three rows;
an ellipsis marks clipped text, which expansion retains in full.
One neutral row separates calls. Success is green, failure red, stopped work
gold, and running/background work uses the tool accent.

Tool cards retain the result reported when a call returned. A Bash launch says
`Started in background · ID`; a running Process check says
`Running at last check · ID`. The tool accent identifies background work at
that point, rather than a live status indicator. `/terminal` shows the current
active processes and removes them from its list when they finish.

Read starts with its returned range; expansion reveals numbered source.
Image and PDF attachments state their view; expansion reveals attachment metadata.
Bash and Process keep a recent three-line preview; failed commands prioritize
up to six error lines. Expansion retains both stdout and stderr. Capture
limits remain visible in either state. A running Bash call shows the
[checkpoint](../usage/CHECKPOINTS.md) recorded before it, such as
`checkpoint cp-42`. Saves are otherwise quiet: a warning appears only when a
checkpoint problem needs attention, and expansion adds the call's
`Checkpoint cp-41 → cp-42` points. Checkpoint calls show a three-line preview
of their labeled result rows. Completed previews stay visible when
the next call starts. Elapsed time stays in the running result or expanded
details, without a right-aligned status column.

Update and Write show contextual diffs with three surrounding source lines,
original coordinates on removals and resulting coordinates on additions and
context. Syntax colors remain above full-row red/green surfaces. Wrapped code
keeps an empty number gutter. Up to 14 logical diff rows remain open; larger
changes preview both sides of the first hunk and state the hidden row count.
Expansion reveals all retained hunks; collapse restores the preview. Empty
creations, no-op edits, final newlines and line-ending conversions have
explicit text. Failed file calls never display a success diff.

Captured changes persist as local session events, outside provider messages
and tool results. Replay uses those captured coordinates and content, without
rereading the file. Old sessions show replacement snippets without inventing
file coordinates; writes with unavailable previous text use a neutral content
preview. Size and alignment limits also use this fallback without affecting
the file operation. See [Tools](../usage/TOOLS.md) for the execution boundary.

`Ctrl+O` expands or collapses every call in the latest finished execution turn.
Opening scrolls to its first tool; closing returns to the final answer. It is
ignored while a turn runs, and a new accepted prompt resets the old expansion.
No tool title opens details individually. See [Composer and controls](COMPOSER.md)
for the complete interaction map and [Tools](../usage/TOOLS.md) for tool behavior.

## Panels and feedback

Commands, selectors, information, and process views share the user surface.
At 12 columns or wider, content has two-cell side margins. The selected row
uses a lighter surface and accent focus marker; current values are identified
in descriptions rather than by focus alone. Delete and discard confirmations
turn the target name red until confirmed or canceled.

Notices sit on the terminal background at the left edge: italic muted text
for information, gold for warnings, and red for errors. The activity line
uses four animated dots, a bold phase label, and muted elapsed time. Queue
previews appear above notices and activity. The context indicator stays in
the muted footer style, including below an open panel.
