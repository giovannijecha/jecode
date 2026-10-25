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
| `EMPHASIS` | `#DCE4F0` | Tool targets, level 2 Markdown headings and strong text. |
| `CODE_TEXT` | `#C9D1D9` | Unhighlighted fenced code and inline code. |
| `MUTED` | `#9AA4B2` | Descriptions, footer, quotes, link destinations, information and notice details. |
| `TOOL_DIM` | `#879AAA` | Tool `⎿` guides, activity time, column headers and command groups. |
| `ACCENT` | `#7AA2F7` | Jecode mark, composer rules, focus, matches, links, level 1 headings and reading tool names. |
| `TOOL_ACCENT` | `#99B8D4` | Running and background tool markers, and command tool names. |
| `KEYWORD` | `#BB9AF7` | Code keywords and file-change tool names. |
| `GOOD` | `#9ECE6A` | Successful tools, added lines and code strings. |
| `BAD` | `#EF8B8B` | Failed tools, destructive choices and removed lines. |
| `WARNING` | `#E0AF68` | Warnings, stopped tools and code numbers. |

Weight and slant are applied by the component: tool names and Markdown
headings are bold. Links use the accent with underline. The palette does not replace the terminal's global colors.

## Surfaces

| Surface | Colour | Current use |
| --- | --- | --- |
| Terminal | Inherited | Conversation and result notices. |
| `USER_SURFACE` | `#282E39` | Sent requests, warning and error banners, and command, selector, information and process panels. |
| `SELECTED_SURFACE` | `#353D4B` | Selected menu/process rows and Back to bottom badge. |
| `CODE_SURFACE` | `#20252C` | Fenced code and inline code. |
| `DIFF_ADDED_SURFACE` | `#1E2E23` | Added fragments; full text for new or unmatched lines. |
| `DIFF_REMOVED_SURFACE` | `#312026` | Removed fragments; full text for unmatched lines. |
| `CURSOR` | `#1A1B26` on `#C0CAF5` | Software caret, including in shaded panels. |

## Conversation and tool cards

The conversation has one neutral row between visible blocks. Sent user
messages have a two-cell indent and shaded padding; assistant Markdown joins
soft-wrapped lines into paragraphs and keeps blank rows. Headings differ by
level and keep one row before the text they introduce. Fenced code spans the
full row width with one cell of padding; inline code sits on the same surface.
See [Markdown responses](MARKDOWN.md) for the display subset.

Every call uses `● Tool(target)` and an indented `⎿` result. The display
names are Read, Skill (a `read` of a skill's `SKILL.md`), Update (the `edit`
tool), Write, Bash, Process, Checkpoint, Web and Browser. The `●` marker
carries the status: success green, failure red, stopped or timed-out work
gold, and running or background work the tool accent. The bold name and its
parentheses carry the tool's family: reading tools (Read, Skill, Web, Browser)
use `ACCENT`, file changes (Update, Write, Checkpoint) `KEYWORD`, and commands
(Bash, Process) `TOOL_ACCENT`; the target stays `EMPHASIS`. One neutral row
separates calls.

Titles wrap on words and continue under the open parenthesis, or two cells in
when fewer than 24 columns would remain. Each command line takes its own row.
A compact title keeps three rows and closes with `…)`; expansion retains it in
full. Result continuations and output use four cells.

The result names what came back as `Subject · detail`:

| Call | Result |
| --- | --- |
| Running | A fixed `●` title marker pulses through `TOOL_PULSE` every 240 ms; the result row keeps the elapsed time and any note, such as the [checkpoint](../usage/CHECKPOINTS.md) `checkpoint rc-ID` recorded before a Bash call. |
| Read | `Read N lines`, with `from line N` past the start; byte reads, image and PDF views and attachments say so. |
| Update, Write | `+N −M lines` in green and red, or explicit text for empty creations, no-op edits, final newlines and line endings. |
| Bash, Process | Collapsed, the last stdout line and the duration from one second. Expanded, `Done in …`. |
| Failed command | `Exit N` in red with the first error line, then two more error lines. |
| Stopped command | `Cancelled after …` or `Timed out after …` in gold, with `partial output` and its last three lines. |
| Background | `ID running` with the latest output line; Process reports `stopping`, `stopped` or `status unknown`. |
| Web | The page title, size and host. |
| Browser | Where it landed or what it saved, such as `Screenshot 1280×720 · path`. |
| Error | The error in red; a missing path reads `Not found`. |

Tool cards retain the result reported when a call returned. `/terminal` shows
the current active processes and removes them from its list when they finish.

Collapsed cards skip blank output lines and count the rest as `… N more
lines`. The last card with hidden lines in the latest finished turn names
`Ctrl+O expand` as a keycap on that row. Expansion shows the exact output,
with stdout and stderr labeled when both exist, numbered source for reads,
raw Web and Browser results, and attachment metadata. Capture limits stay
visible in either state as upright muted notes, such as `stdout trimmed to the
last 64 KiB of 89 KiB · full output saved`. Checkpoint saves are otherwise
quiet: partial or missing coverage shows a warning, and expansion adds the
call's `Checkpoint rc-ID` reference. Checkpoint calls preview three labeled
result rows. Completed previews stay visible when the next call starts.

Consecutive successful line reads share one card: `● Read(N files)` with the
file names, the parent folder when two names match, and the total lines.
Pages of one file keep its path as the target and count the reads. The list
keeps two rows and names `Ctrl+O list` when it is the last hidden card;
expansion gives each read its own `path · N lines` row. Failures, edits and
commands start a card of their own.

Update and Write show contextual diffs with three surrounding source lines,
original coordinates on removals and resulting coordinates on additions and
context. Context text is `TOOL_DIM`. Changed fragments use bold `BAD`/`GOOD`
on the matching removed/added surface, while unchanged code keeps its syntax
color. Adjacent removed/added runs pair by position; pure additions or
removals highlight their full text. Wrapped code keeps an empty number gutter.
Up to 14 logical diff rows remain open; larger
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
Opening and closing preserve the reading position; following the tail stays at
the tail. It is ignored while a turn runs, and a new accepted prompt resets the old expansion.
No tool title opens details individually. See [Composer and controls](COMPOSER.md)
for the complete interaction map and [Tools](../usage/TOOLS.md) for tool behavior.

## Panels and feedback

Commands, selectors, information, and process views share the user surface.
At 12 columns or wider, content has two-cell side margins. Headers put the
title in bold `STRONG` with muted context and aside; a `SCANNER_REST` rule or
section tabs with an `ACCENT` underline sit below. Search, key and number
inputs, and process output, use `CODE_SURFACE` insets. The selected row uses a
lighter surface and an accent `›` marker; current values carry a green `●`.
Column headers and command groups use `TOOL_DIM`. Key hints are keycaps on
`SELECTED_SURFACE`, and long lists show a `┃` thumb on a `│` track. Delete
and discard confirmations replace the key hints in red and turn the target
name red until confirmed or canceled.

Notices follow their lifetime. A result is a line on the terminal background
with no icon and both subject and detail in `MUTED`;
a change reads `Subject old → new · scope` with the new value
strong. Warnings and errors are banners on the user surface with a `▌`
stripe and subject in `WARNING` or `BAD`; an error's cause after a colon
keeps `BAD`, while trailing context stays muted. Their next step is a keycap
at the right. Progress has no row of its own: it rides the composer's upper
rule, which carries the activity scanner (`▰` lit, `▱` resting), muted
elapsed time while a turn runs, and the progress subject in `STRONG`, or
`WARNING` with a countdown while a request waits to retry. Without progress,
the rule names the current turn phase; `Stopping` takes priority when cancellation
is requested. Queue previews appear above notices and the
composer. The footer stays muted, including below an open panel; its context
meter draws filled `━` cells in `SCANNER_LIT` on a `─` track in
`SCANNER_REST`, and both meter and share turn `WARNING` near auto-compaction.
