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
| `EMPHASIS` | `#DCE4F0` | Level 2 Markdown headings and strong text. |
| `CODE_TEXT` | `#C9D1D9` | Unhighlighted fenced code and inline code. |
| `MUTED` | `#9AA4B2` | Secondary text, footer, quotes, link destinations, information and notice details. |
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
| `DIFF_ADDED_SURFACE` | `#1E2E23` | Full tint on added rows beyond the four-cell indent. |
| `DIFF_REMOVED_SURFACE` | `#312026` | Full tint on removed rows beyond the four-cell indent. |
| `DIFF_ADDED_STRONG` | `#2C4A33` | Paired changed fragments on added rows. |
| `DIFF_REMOVED_STRONG` | `#523039` | Paired changed fragments on removed rows. |
| `CURSOR` | `#1A1B26` on `#C0CAF5` | Software caret, including in shaded panels. |

## Conversation and tool cards

The conversation has one neutral row between visible blocks. Sent user
messages have a two-cell indent and shaded padding; assistant Markdown starts
with a neutral `●` in `BODY`, then hangs every later row two cells in. It joins
soft-wrapped lines into paragraphs and keeps blank rows. Headings differ by
level and keep one row before the text they introduce. Fenced code spans the
indented response width with one cell of padding; inline code sits on the same surface.
See [Markdown responses](MARKDOWN.md) for the display subset.

Individual calls use `●` and an indented `⎿` result. The display
names are Read, Skill (a `read` of a skill's `SKILL.md`), Update (the `edit`
tool), Write, Bash, Process, Checkpoint, Web and Browser. The `●` marker
carries the status: success green, failure red, stopped or timed-out work
gold, and running or background work the tool accent. Bold names carry their
family colour: reading tools (Read, Skill, Web, Browser) use `ACCENT`, file
changes (Update, Write, Checkpoint) `KEYWORD`, and commands (Bash, Process)
`TOOL_ACCENT`. Read, Update and Write keep family-coloured parentheses but use
`BODY` paths; routine parentheses and targets also use `BODY`, except the
muted Browser action. One neutral row
separates visible blocks.

File titles wrap on words and continue under the open parenthesis, or two cells
in when fewer than 24 columns would remain.
A compact file title keeps three rows and closes with `…)`; expansion
retains it in full. Routine titles stay on one row. Bash and Process prefer optional canonical `description`
arguments for `● Bash  description`; without one they show the command or
target. The command follows at `  ⎿ $ `, shell-coloured and limited to two
compact rows; Ctrl+O shows it in full. Process adds a bold tool-accent ` &`.
Browser uses `Browser(action)` with the action muted and a JavaScript-coloured
payload below, also limited to two compact rows. Result continuations and
output use four cells.

The result names what came back as `Subject · detail`:

| Call | Result |
| --- | --- |
| Running | A fixed `●` title marker pulses through `TOOL_PULSE` every 240 ms; the result row keeps the elapsed time and any note, such as the [checkpoint](../usage/CHECKPOINTS.md) `checkpoint rc-ID` recorded before a Bash call. |
| Read | One compact result: `9 lines · from line 33 of 212` when the total is known, or `9 lines · from line 33 · more from 42` when another page exists and the total is unknown. At narrow widths the start clause yields to `9 lines · more from 42`; split lines retain `continues at byte N`. No separate compact pagination notice. Skill says `Loaded skill`. |
| Update, existing-file Write | `+N −M lines` in green and red, or explicit text for no-op edits, final newlines and line endings. |
| New Write | `New file · N lines`, followed by the first six numbered source rows; Ctrl+O shows every row. |
| Checkpoint | One quiet result with the hidden row count; Ctrl+O opens the labeled details. |
| Bash, Process | A muted lead gives the last stdout line and duration; `TOOL_DIM` adds `+N lines · no checkpoint` inline. Expanded cards retain full output. |
| Failed command | A decoded `BAD` message on at most two rows, a muted position and up to two more `BAD` stderr rows. |
| Stopped command | A muted `Cancelled after …` or `Timed out after …` lead with an inline hidden-line count. The status dot stays gold; Ctrl+O opens the retained partial output. |
| Background | A tool-accent `ID running` lead with the latest output line; Process reports `stopping`, `stopped` or `status unknown`. A Process list shows one compact count; expansion opens the list. |
| Web | The page title, size and host. |
| Browser | Where it landed or what it saved, such as `Screenshot 1280×720 · path`. |
| Error | The decoded message in `BAD`; a missing path reads `Not found`. |

Tool cards retain the result reported when a call returned. `/terminal` shows
the current active processes and removes them from its list when they finish.

Compact routine cards skip blank output lines and fold the hidden count into
the `+N lines` result suffix. New files use a `+N lines` source notice and
long diffs name hidden hunks and rows. The last
hidden card or Work chapter in the latest finished turn names
`Ctrl+O expand` as a keycap on its summary row; a chapter instead keeps that
keycap at the right edge of its title. Expansion shows the full
command and exact output, blank rows included. Stdout uses `CODE_TEXT`,
successful stderr `MUTED`, and stream labels `TOOL_DIM` when both exist.
Bash and Process close with `exit N · duration` if an exit code exists, or
the duration alone. Text Read output stays numbered; an expanded image Read
uses an `Opened image` fact with dimensions and bytes when available, without
attachment JSON. Web and Browser retain raw results and attachment metadata.
Capture limits stay
visible in either state as upright muted notes, such as `stdout trimmed to the
last 64 KiB of 89 KiB · full output saved`. Checkpoint saves are otherwise
quiet: partial coverage shows a warning; missing coverage on routine calls
joins the compact result line, while file cards keep their warning. Expansion adds the
call's `Checkpoint rc-ID` reference. Checkpoint calls show one compact result;
expansion opens the labeled rows. Standalone previews stay visible when the next call starts.

Two or more adjacent `read`, `bash`, `process`, `web` and `browser` calls share a
Work chapter when their normalized `work` labels and execution turn match. A
single eligible call keeps its normal compact card and full expansion. Skill reads,
assistant narration and other tools break the chapter. Missing or blank labels
display as **Activity**, including in older sessions. The displayed label
collapses whitespace to one line and is ellipsized to at most 60 terminal cells;
the saved label stays unchanged and grouping compares the full normalized label
before display truncation. The chapter has one status dot, a bold tool-accent
label, a dim call count and an optional `BAD` failed count, without parentheses.
An available `Ctrl+O expand` keycap sits at the right edge of the title row;
the title and facts clip before it, and it never adds a row.
An `⎿` summary counts successful work: unique files read, other reads,
web calls, browser captures and actions, commands and
process calls. It also counts running, background and stopped calls when
needed, but omits failures already counted in the title. The facts row
always remains, reading `0 completed` when it has no other fact. Each Bash
or Process member keeps one compact `Name  description` row, falling back to
the command. Running time and a failed `exit N` sit inline; process IDs,
captured output and raw HTTP logs wait for Ctrl+O. A tool error can add two
bounded message rows only for non-command members; Bash and Process failures
stay on one member row. A former singleton can retain an outcome receipt at
the chapter foot when needed to preserve compact height. A failed command uses
its first nonempty stderr line or captured-output count instead of repeating
the exit status already shown on its member. Other exceptional calls
keep a primary row and bounded errors when useful. Ordinary Read, Web and
Browser calls share one activity row at the bottom. It names the latest
call without dedicated error detail, with a two-space gap after the name and
freezes its timer when done. Failed calls stay in their exception rows and do
not repeat in activity. Stopped and background states remain named. A
compact chapter never loses height, including when a new turn starts; it uses
meaningful content or the retained activity receipt instead of empty internal
padding. A second related call turns an eligible singleton into a chapter.
Every Bash command keeps its one compact row after success, even in older
turns. Full output remains available through latest-turn `Ctrl+O`.
The summary does not add durations across calls. `Ctrl+O` shows each
call's existing complete card inline for the latest finished turn, then restores
the compact view; it adds no panel or separate shortcut.

Update and existing-file Write show contextual diffs with three surrounding
source lines, original coordinates on removals and resulting coordinates on
additions and context. Context is dim and unshaded. Each changed row is tinted
from its coordinate through the right edge; the first four cells stay on the
terminal surface. Paired changed fragments alone use bold `BAD`/`GOOD` on the
stronger removed/added surface. Unpaired code keeps its syntax colours on the
row tint without whole-line bold. Adjacent removed/added runs pair by
position. Wrapped code keeps an empty number gutter. Up to 14 logical diff
rows remain compact; longer changes report a dim hidden fact such as
`+1 hunk · 6 rows`. Expanded hunks are separated by a dim ellipsis under the
sign column.
Expansion reveals all retained hunks; collapse restores the preview. Empty
creations, no-op edits, final newlines and line-ending conversions have
explicit text. Failed file calls never display a success diff.

Captured changes persist as local session events, outside provider messages
and tool results. Replay uses those captured coordinates and content, without
rereading the file. Old sessions show tinted replacement snippets without
inventing file coordinates; writes with unavailable previous text use a neutral content
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
