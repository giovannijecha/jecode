# TUI theme

This is the visual reference for contributors changing the TUI: colour tokens,
the screen grid and how each element is drawn. For how to use the interface, see
[Terminal interface](../usage/INTERFACE.md) and [Tool activity](../usage/TOOL_ACTIVITY.md).

The interactive TUI runs on the terminal's alternate screen. Jecode never paints
a background, in either theme: cells without a surface keep the terminal's own
background (SGR 49), and the terminal palette is never changed. Every colour role
has an ink for dark terminals and an ink for light terminals, chosen when the
role is painted, so call sites never depend on the theme. `NO_COLOR` is ignored.
The terminal still chooses the font and how bold, italic, underline and
strikethrough look. Tokens live in
[`src/tui/theme.rs`](https://github.com/giovannijecha/jecode/blob/main/src/tui/theme.rs).

## Choosing the inks

The `appearance.theme` setting (`auto`, `dark` or `light`; see
[Configuration](../reference/CONFIGURATION.md#the-configuration-file)) chooses which
inks are used. `dark` and `light` fix the choice. `auto`, the default, follows
the terminal: the TUI queries OSC 11 once after setup, and a complete
`rgb:R/G/B` reply with relative luminance above 0.5 selects the light inks. The
query accepts BEL or ST termination and one to four hex digits per channel. If no
valid reply arrives within 800 ms on Windows, the native console adapter uses
`GetConsoleScreenBufferInfoEx` to read the main screen's background palette
entry, captured before entering the alternate screen. Without any answer the
terminal counts as dark. The answer only picks inks; it never adds a background.

`Settings › Theme`, the sixth of the nine Settings rows, saves the choice once a
default model exists; before that it lasts for the session. It applies at once,
to the current conversation and to new ones, and repaints the screen.

Colour is 24-bit on Windows and on Unix terminals that declare it through
`COLORTERM`, `TERM`, `TERM_PROGRAM` or their own variable. Other terminals, such as
Terminal.app before macOS 26 or a session over SSH without `COLORTERM`, get the
nearest entry of the 256-colour palette: its cube and grey ramp, never the first 16
colours a theme can change. The dark tints that carry meaning would all turn grey
there, so diffs use palette greens and reds (22, 52) and the shell, coordination and
failure badges keep a hue (25, 23, 88). Setting `COLORTERM=truecolor` asks for 24-bit
colour on a terminal Jecode does not recognize. Tests hold text on every surface in the
palette at 4 or more, and 3 for `MUTED` on an added row; `DIM` line numbers reach about
2.3 there.

## Tokens

Text roles, with the ink for each kind of terminal:

| Role | Dark | Light | Use |
| --- | --- | --- | --- |
| `STRONG` | `#EFF0F3` | `#191B1D` | Requests, drafts, the product name, titles, option names, field values, keys, the footer percent, labels on quiet badges. |
| `LIT` | `#DACEFF` | `#48198A` | The caret's blinking `_`, background badge names, command names in Work chapters, `new output`, attachment chips. |
| `BODY` | `#D1D2D6` | `#2C2E31` | Assistant prose, tool targets, diagnostics. |
| `CODE_TEXT` | `#D1D2D6` | `#2C2E31` | Default code, inline code, command output, diff context. |
| `GOOD` | `#7CD992` | `#066F32` | The `Done` and `✓` marks, added lines, `☑`. |
| `ACCENT` | `#C7AFFF` | `#552B9A` | The `>_` mark, `›` and `●` markers, matches, level 1 headings, links, the words `current` and `recommended`, the active tab mark, the gauge fill. |
| `KEYWORD` | `#C8B7FF` | `#553494` | Code keywords. |
| `CODE_STRING` | `#99D3DA` | `#016770` | Code strings. |
| `CODE_NUMBER` | `#E6B2CF` | `#943171` | Code numbers. |
| `WARNING` | `#EDC06B` | `#875500` | Retry, stopped and warning glyphs, `paused`, the gauge and its percent near compaction. |
| `BAD` | `#F48684` | `#B32228` | Failures, removed lines, destructive questions. |
| `MUTED` | `#A4A6AA` | `#515256` | Secondary facts, field labels, link hosts, quotes, list markers, the model and effort in the footer; `DIM` text on a selected row. |
| `DIM` | `#8C8E92` | `#616367` | Status-row facts, key labels, the footer path, tree gutters, group headings, line numbers, placeholders, struck text. |
| `FAINT` | `#6D6F72` | `#84868A` | Marks only, never text: table borders, scrollbar and gauge tracks, thematic breaks. |
| `HAIRLINE` | `#3B3D40` | `#CFD1D5` | Decorative rules and tab dividers, which carry no meaning. |

Contrast is measured against typical terminal backgrounds, since Jecode never
sets one: `#0C0C0C`, `#17161F` and `#1E1E1E` for the dark inks, `#FFFFFF`,
`#FAFAFA` and `#FDF6E3` for the light inks. Tests in `theme.rs` hold these
floors on each: `STRONG` 13, `BODY` 11, `ACCENT` 8.5, `MUTED` 6.5, `DIM` 5,
`GOOD`, `BAD` and `WARNING` 4.5, and `FAINT` 3. `HAIRLINE` has no floor.
Terminals with a lighter dark background, such as One Half Dark (`#282C34`),
bring every role but `GOOD`, `BAD` and `WARNING` below those floors.

The activity shimmer uses `S0`, `S1` and `S2` over `S_BASE`:

| Role | Dark | Light |
| --- | --- | --- |
| `S0` | `#C7AFFF` | `#7250BA` |
| `S1` | `#DACEFF` | `#522797` |
| `S2` | `#FFFFFF` | `#27124B` |
| `S_BASE` | `#75689C` | `#9C8DCA` |

Tool badge labels use `BADGE_TEXT` `#EFF0F3` on both. The software caret is
`#0C0C0C` on `#EFF0F3` for dark inks and `#FFFFFF` on `#191B1D` for light ones.

Surfaces mean something: a request, a selection, code, a changed line. They are
the only fills Jecode paints.

| Surface | Dark | Light | Use |
| --- | --- | --- | --- |
| `BASE` | terminal default | terminal default | Every cell without a surface, in both themes. |
| `USER_SURFACE` | `#292A2F` | `#E6E8EC` | Sent requests. |
| `SELECTED_SURFACE` | `#3A3B42` | `#D8DBE0` | The focused option row. |
| `CODE_SURFACE` | `#212226` | `#EEF0F4` | Code cards, inline code, process output. |
| `DIFF_ADDED_SURFACE` / `DIFF_REMOVED_SURFACE` | `#1A2A1F` / `#312026` | `#DCF2DF` / `#FFE4E4` | Added and removed diff rows. |
| `DIFF_ADDED_STRONG` / `DIFF_REMOVED_STRONG` | `#2C4A33` / `#523039` | `#C8ECCE` / `#FFD5D5` | Paired changed fragments. |

Tool badges carry the family colour when done and the state otherwise. The
family surfaces and their labels are the same in both themes; only the quiet
surface follows the theme, and its label with it.

| Badge surface | Dark | Light | Use |
| --- | --- | --- | --- |
| `BADGE_READ` | `#4852A0` | `#4852A0` | Read, List, Search, Skills, HTTP, Browse; the Read level chip. |
| `BADGE_EDIT` | `#7F3C93` | `#7F3C93` | Update, Save; the Workspace level chip. |
| `BADGE_SHELL` | `#036387` | `#036387` | Bash, Exec, Proc; the Full level chip. |
| `BADGE_COORD` | `#226558` | `#226558` | Agents, Todo, Ask user and Main. Collaborator numbers follow the badge. |
| `BADGE_IDLE` | `#3A3B42` | `#D8DBE0` | Running and background badges, and the finished MCP CALL, LSP CALL, WORK and other tools without a family (label in `STRONG`); attachment chips and the footer `AGENTS` badge. |
| `BADGE_BAD` | `#7A2E3A` | `#7A2E3A` | Failed calls. |
| `BADGE_WARN` | `#6B5222` | `#6B5222` | Stopped or timed-out calls. |

Tests hold every label at 4.5 or more on the surface it is drawn on. A running
badge's shimmer rests on `DIM`, which holds 3 on `BADGE_IDLE`.

## Screen and grid

From the top: the header, the conversation, one gap row, then the dock. The
header is the start of the conversation and scrolls away with it; the dock is
anchored to the bottom. Markers (`›`, `●`, `✕`) sit at column 0, text starts at
column 2, and right-hand facts end two cells before the edge, except the footer,
which runs from edge to edge. One blank row
separates conversation blocks. Tool badges take eight cells (see
[Tool rows](#tool-rows)); attachment chips and the level chip have one cell of
padding.

The dock holds, top to bottom: the queued row, the status row, the composer
(top rule, draft, bottom rule) and the footer or key hints. A sheet replaces the
composer; see [Panels](#panels). Rules are `HAIRLINE` in every state. An empty
draft shows its placeholder in `DIM`. While a draft exists and the composer
holds the keys, the bottom rule names `Ctrl+G commands` on the left, in `MUTED`
and `DIM`; it leaves first when the row is narrow. The level chip sits on the
right. See [Terminal interface](../usage/INTERFACE.md).

After the first prompt, the header reads `>_ jecode <version>`, with `>_`
bold `ACCENT`, `jecode` bold `STRONG` and the version `MUTED`, shortened from
the right. The model, effort and folder sit in the footer, so each appears
once on screen. The footer shows the folder in `DIM` on the left; on the right
the model and effort are `MUTED` text with no chip, two cells, then the context
gauge: eight cells with the fill in `ACCENT` and the track in `FAINT`, and the
percent in bold `STRONG`. Within three points of the compaction threshold the
fill and the percent turn `WARNING`.

With active collaborators or pending review, the same footer row replaces the
folder with a quiet `AGENTS` badge (`LIT` on `BADGE_IDLE`, `BADGE_TEXT` on
`BADGE_BAD` when work failed) and readable state counts. Failed work uses the
existing error roles; uncertain outcomes use `WARNING`. An accent `/agents` hint
appears when space permits. The context gauge remains and model details yield
at narrow widths. The normal footer returns once nothing needs attention; the
dock keeps its height.

An empty conversation shows a centred startup block above the dock. Everything
sits inside the 49-cell width of the mark, which is the `>_ jecode` scanline
mark in `ACCENT` with its blinking `_` in `LIT`:

- the version on the left, and on the right the available skill count and
  whether this folder has an `AGENTS.md`;
- for saved conversations, `Recent in this folder` in `DIM` with `/resume all`
  on the right, then up to three titles in `BODY` with compact ages such as
  `2h` in `MUTED`, even without a model (the dock keeps its `No model` warning);
  with neither a model nor saved conversations, `Start here` in `DIM` and three commands,
  `/model`, `/permissions` and `/help`, in `STRONG`, each with a `MUTED` fact;
- the keys `/ commands`, `Alt+V image` and `Ctrl+Q quit`, with `Ctrl+V image` on
  macOS.

Deleting a conversation through `/resume` updates the recent list at once.

The mark needs at least 53 columns. As the available height shrinks, the key row
disappears first, then the mark becomes three compact rows. Smaller screens use
the one-line header, followed by the recent conversations or the first-run
commands at column 2 and no keys. The first prompt removes the startup block; the
dock keeps its normal layout throughout.

## Status row and motion

The status row sits above the composer and is always one row of text. Every
state has the same shape: a two-cell glyph, one space, the label, `DIM` facts
joined by ` · ` and dropped from the right, then an optional key ending two
cells from the edge, which drops after the facts. The key is bold `STRONG` with
its label in `DIM`.

| State | Glyph | Label | Facts |
| --- | --- | --- | --- |
| Idle | — | empty row | — |
| Working | blinking `>_` | activity, shimmering | `Esc interrupt · turn time · ↑ input tokens · ↓ generated tokens` |
| Retry | blinking `>_` in `WARNING` | `Retrying in 4s` in `WARNING` | `Esc interrupt · turn time · attempt N · reason` |
| Done | static `>_` in `GOOD` | `Done` | `turn time · N tools · ↑ input tokens · ↓ generated tokens`, then the serving provider when reported |
| Stopped | `■` in `WARNING` | `Stopped` | `time`, then `prompt restored` if the message came back, otherwise `your next message continues from here`; an autosave error if restoring could not be saved; `draft paused` if a draft was parked |
| Failed | `✕` in `BAD` | `Failed` | `time`, then `prompt restored` if the message came back; an autosave error if restoring could not be saved; `draft paused` if a draft was parked; `/settings update key` after a rejected key |
| Result | `✓` in `GOOD` | subject, such as `Copied` | detail; a setting reads `old → new` with the new value `STRONG` |
| Warning | `▲` in `WARNING` | subject | detail and a fixing key |
| Error | `✕` in `BAD` | subject | detail and a fixing key |
| Waiting | static `?` in `ACCENT` | `Waiting for you` or `Waiting for permission` | none; `/ask answer` as the key for a question whose panel is closed |

`Esc interrupt` appears only while Esc would stop the turn. The Working label
is the running call's `description` when given, else a verb and a short
target: `Reading`, `Editing`, `Writing` a file; `Running` a command;
`Starting` an `exec` program; `Fetching` or `Browsing` a host;
`Loading skill`. Between calls it names the phase, such
as `Thinking` or `Working`; `Stopping` takes priority
after cancellation. Compaction and attachment progress take the label while
they run.

One motion frame is 100 ms. The caret keeps `>` in `ACCENT` and blinks `_` in
`LIT`, 400 ms on and 400 ms off. One shimmer band, `S0 S1 S2 S1 S0` moving one
cell per frame, runs over `S_BASE` through the status label and over `DIM`
through running badges and running Work chapters.

## Conversation

A sent request is one band of full-row `USER_SURFACE`, with no padding rows:
`› ` in bold `ACCENT` at column 0 and the text in `STRONG` at column 2, wrapped
rows hanging under it. Request text stays plain, apart from
attachment labels, which become ` 1 Image ` or ` 2 wake.log ` chips in `LIT`
on `BADGE_IDLE`.

An answer opens with `●` in `ACCENT`, prose at column 2; see
[Markdown responses](../reference/MARKDOWN.md). Reasoning stays hidden. A failed turn keeps
its diagnostic as the turn's last row: `✕` in bold `BAD`, then the text in
`BODY`.

## Tool rows

Each call opens with a badge, then the target one space after in `BODY`. The
badge fills exactly eight cells with its name centred in bold. Tool labels have
even widths of at most eight cells, leaving equal padding on both sides. Every
target starts on column 9, where diff code starts when line numbers have two
digits. Unrecognized tool names keep the same filled footprint, clipping long
names and rounding odd-width padding to a cell.

There is no separate status dot: a finished call keeps its family colour, a
running one shimmers on `BADGE_IDLE`, a background one shows a `LIT` name on
`BADGE_IDLE`, a failed one turns `BADGE_BAD` and a stopped one `BADGE_WARN`.
Finished badges with a family colour use `BADGE_TEXT`; the quiet ones (MCP CALL,
LSP CALL, WORK and tools without a family) use `STRONG`. The names are READ,
SKILLS (a `read` of a skill's `SKILL.md`), UPDATE, SAVE, BASH, EXEC, PROC, HTTP,
BROWSE, LIST, SEARCH and AGENTS, plus SEQUENCE on a refused `steps` call. List and
Search use the read-family surface; an optional description keeps the
concrete target on the following tree row. A permission request names the main
agent with a centred `MAIN` badge on the coordination surface.

Successful writes to existing files show UPDATE when their retained change
confirms the earlier file, including whole-file replacements. Captured new
files show SAVE. Pending or failed writes, and writes without a retained
change, keep SAVE. The same rule applies when expanding a card or restoring
its saved history; the renderer does not inspect the current file to choose
the earlier operation's label.

Collaboration creation, sent/received assignments, returned reports, review and
closure use immutable chronological receipts. Every collaborator badge reads
`AGENTS`, with the display number outside its surface, such as `#1`. The number
does not resize the badge; raw internal IDs such as `a1` remain hidden. Current
state belongs to `/agents`. Inspection, wait, diff and integration
calls retain standard `AGENTS` tool cards with their action and target; calls
already represented by receipts avoid duplicate cards. Errors remain visible.

Result rows hang under the title on a `DIM` tree: `│` on every row, `└` on the
last, wrapped continuations included. Diffs run from column 2 without a tree.

File titles show the path, wrap on words under it, or at column 2 when fewer
than 24 columns would remain, and keep three rows with a closing `…` until
expanded. When the result fits beside a one-row title it joins that row;
otherwise it hangs below. Bash, Exec and Process prefer the optional
`description` argument as the target; the command then follows on the tree as
`$ command`, shell-highlighted, or `> program args` for Exec, limited to two
compact rows. Without a description the command or process target is the
title's target. A Bash or Process title ends in a bold `LIT` ` &` only while
its process runs in the background. Browser shows its action as the target and
the JavaScript-coloured payload below; an action without one, such as
`snapshot` or `stop`, opens its result on the first tree row, never on an
empty one.

The result names what came back as `Subject · detail`, `MUTED` once finished
and `DIM` while running:

| Call | Result |
| --- | --- |
| Running | The elapsed time, after `Running` for routine calls, and any note. |
| Read | `9 lines · from line 33 of 212`, or `9 lines · from line 33 · more from 42` when the total is unknown. Narrow rows drop the start clause; split lines keep `continues at byte N`. Skill reads `Loaded skill · …`. |
| List, Search | Entry or matching-line counts, searched files, errors and `more from N` when paged. Expanded cards show structured paths, source context and exclusions. |
| Update (`edit` or an existing-file `write`) | `+N −M lines` in `GOOD` and `BAD`, or explicit text for no-op edits, final newlines and line endings. |
| New Write | `New file · N lines`, then the first six numbered source rows. |
| Bash, Exec, Process | The last output line with the duration from one second, `Done in 1.2s`, or `Done`; `+N lines` follows in `DIM`. |
| Failed command | `Exit N · diagnostic` in `BAD` on at most two rows, a muted position, up to two more `BAD` stderr rows. Recognized Node uncaught-exception headers give way to the actual error. |
| Stopped command | `Cancelled after 6.0s` or `Timed out after …`, `· partial output` when some was captured, and the hidden line count. |
| Background | `p3 running · latest line`; Process reports `stopping`, `stopped` or `status unknown`. A process list shows the total and the running count. |
| Web | The page title, size and host. |
| Browser | Where it landed or what it saved, such as `Screenshot 1280×720 · path`. |
| Error | The decoded message in `BAD`; a missing path reads `Not found`. |

Compact cards skip blank output lines and fold the hidden count into the
result. Node exception logs render terminal controls as a readable transcript;
Ctrl+O retains the complete diagnostic, including the runtime header. Saved
output bytes and the program or wrapper's reported exit code remain intact.
Capture limits stay visible as muted notes, such as `stdout trimmed to
the last 64 KiB of 89 KiB · full output saved`.

`Ctrl+O` expands every call of the latest finished turn into its full card:
the full command and exact output, numbered source for Read, all diff hunks,
and `exit N · duration` for commands. Stdout uses `CODE_TEXT`, successful
stderr `MUTED`, and stream labels `DIM` when both streams exist. An expanded
image Read shows an `Opened image` fact with dimensions and bytes; Web and
Browser keep their raw results. The last card of that turn with hidden
content carries the key as text, `Ctrl+O` in bold `STRONG` and `expand` in
`DIM`, ending two cells from the edge of the row reporting it, or on a row of
its own when that row is full; expanded, the turn's first card names `collapse`
on its title. Only the latest turn carries it. Tool cards keep the result
reported when the call returned; `/process` shows the live process list.

### Work chapters

Two or more adjacent Read, List, Search, Bash, Exec, Process, Web, Browser, MCP and LSP
calls, collaboration events and agent list, inspect, wait and diff calls share a
Work chapter when their normalized `work` labels and execution turn match.
Skill reads, assistant text and other tools break the chapter; a single call
keeps its normal card. A missing label displays as **Activity**; the label is
collapsed to one line and ellipsized to 60 cells for display only.

The title is a WORK badge, the label in bold `STRONG`, `N calls` in `DIM`
and `· N failed` in `BAD`. The badge carries the chapter's state: `STRONG` on
`BADGE_IDLE` when done, shimmering while running, red or amber on failure or
stop. When the chapter carries the turn's `Ctrl+O` key, it sits at the
title's right edge and the title clips before it.

One tree runs from the title to the chapter's last row. The first row counts
successful work: unique files read, listings, searches, other reads, web calls, screenshots,
snapshots, browser actions, commands and process calls, plus running,
background and stopped calls; it reads `0 completed` when it has nothing else.
Each command keeps one row with its family-coloured name and description or
command, with running time or a failed `exit N` inline. Other failed or
unusual calls keep a row and bounded errors. Up to four latest completed
ordinary Read, List, Search, Web or Browser calls stay at the foot in
source order, with their times frozen when done. Running ordinary calls take
these rows while active. A chapter that replaced a single card keeps at least
that card's height. Expanded, the title's tree closes on its facts, every call
follows as its full card, and the foot rows are dropped.

While several calls run together, each running Read, List, Search, Web or
Browser member keeps a compact target and elapsed-time row in source order.
The chapter facts count running calls; the latest completed ordinary call
remains last. As results arrive, the corresponding running rows become
completed foot rows without shrinking the chapter. Earlier completed rows
leave the four-row window as further calls start. The chapter updates in place
even when completion order differs from source order. The status row reads
`N tools running` while more than one call
is active and returns to the remaining call's normal activity label when one
is left. `Ctrl+O` still applies to the latest finished turn.

### Diffs

Update and existing-file Write show contextual diffs with three surrounding
lines: original coordinates on removals, resulting coordinates on additions
and context. Rows run from column 2 to the edge; added and removed rows are
tinted, context rows stay on the terminal's background in `CODE_TEXT`. Line numbers are `DIM`,
marks `GOOD` `+` and `BAD` `−`. Paired changed fragments alone use bold
`GOOD`/`BAD` on the stronger surface; other changed code keeps its syntax
colours; fragments snap to whole words, and in markup to whole tags and
entities. Wrapped code breaks at token boundaries, as in
[Markdown](../reference/MARKDOWN.md) code cards, and keeps an empty number gutter.
Collapsed, a context line keeps one row and a changed line two, cut with a
`DIM` `…`; when the changed fragment would fall past them, both partner rows
open with `…` at one shared start about 12 cells before it, so the change
stays visible and aligned; a window's second row hangs two cells, as the
window hides the indentation. A new file's preview keeps two rows per line.
`Ctrl+O` shows every line complete. Up to 14 lines stay
compact; longer changes show the first hunk, shorten long runs to
`… N added lines` and report the rest as `+1 hunk · 6 rows`. Expanded hunks
are separated by a `DIM` `⋮`. Empty creations, no-op edits, final newlines
and line-ending conversions have explicit text; failed file calls never show a
diff.

Captured changes persist as local session events, outside provider messages
and tool results, so replay never rereads the file. Older sessions show tinted
replacement snippets without file coordinates; writes without the previous
text, and changes past the size or alignment limits, use a neutral content
preview. See [Tools](../usage/TOOLS.md) for the execution boundary.

## Panels

A panel is a sheet that takes the composer's place; see
[Terminal interface](../usage/INTERFACE.md#menus-and-panels). From the top, the dock
holds an optional queued row, a `draft kept` row in `DIM` when a draft exists,
the status row, the title rule, an optional field on its own row, a blank row,
tabs or the body, a blank row and the keys. Only `/` completion keeps the
composer, because there the draft is the filter.

The title rides a `HAIRLINE` rule as `Parent › Title · context`: parent and
context `MUTED`, title bold `STRONG`, an aside such as `3 of 12 models` on the
right. Ask user has a rule titled `Question` and a permission request one titled
`Permission`; both end in a blank row instead of a bottom rule.

A field has its label in `MUTED`, two cells, then the value in `STRONG`. It
holds the only caret and wraps under itself, and a secret shows as dots. When
fewer than 18 cells would remain for the value, the label stacks above it. An
empty field shows its placeholder in `DIM`.

Option rows put `›` in `ACCENT` at column 0 on the selected row, which takes
`SELECTED_SURFACE` across the whole row; `DIM` text there reads `MUTED`. Names
are `STRONG` at column 2 with matched characters in bold `ACCENT`; a fact column
follows at the widest name plus two, at most three fifths of the width: `MUTED`
by default, saved values in `BODY`, MCP and LSP health in state colours. Badges and ages end on the right; the current item carries the word
`current` in `ACCENT`, and a recommended answer the word `recommended`. Group
headings are `DIM`. A `┃` thumb in `MUTED` on a `│` track in `FAINT` takes the
last column when the list overflows. Digits `1`–`9` choose among up to nine
options; ten or more are filtered by typing.

Ask answer options use `○` when unselected and `◉` in bold `ACCENT` when
selected, for both single and multiple choice. Each mark takes one cell and
one following space before the label. The `›` marker and `SELECTED_SURFACE`
show cursor focus independently of the selected answers; nothing is preselected.
Multiple choice keeps its Space toggle and Enter send/next keys.

Tabs show names in `MUTED`, the active one bold `STRONG` over an `ACCENT` `━`
on a `HAIRLINE` divider, with an optional mode word on the right. Process output
sits on `CODE_SURFACE` from column 2 to the edge.

Keys are text: the key in bold `STRONG` and its label in `DIM`,
three cells apart. A confirmation replaces the keys with its question in `BAD`,
and the marked row turns `BAD` until confirmed or cancelled.

The commands sheet that `Ctrl+G` opens is titled `Commands` and has a `Filter`
field. It lists the same groups and order as `/`. While a turn runs, a command
that must wait for its end stays listed in `DIM` with `after this turn` as its
fact, and cannot be selected.

## Reading back and exit

While you read older output, the gap row above the dock shows
`↓ N rows below`, with `· new output` in `LIT` once output arrived after you
scrolled away; the arrow is `ACCENT` then, `DIM` otherwise. The key
`Alt+End back to bottom`, as text, ends two cells from the edge; it drops first
when narrow. Layout rebuilds add no activity label in the gap row; the dock's
single status row continues to describe the agent's work.

After the alternate screen closes, the normal screen keeps the terminal's own
colours. The exit summary uses only ANSI magenta for `>_`, bold for `jecode`,
SGR dim for the facts and the default foreground for the resume command; see
[Terminal interface](../usage/INTERFACE.md#quit).
