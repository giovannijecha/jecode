# TUI theme

The interactive TUI runs on the terminal's alternate screen. Cells without a
surface keep the terminal's own background (`BASE` is SGR 49); text, marks and
surfaces use the palette below, which assumes a dark terminal theme. Colour is
always 24-bit; `NO_COLOR` is ignored. The terminal still chooses the font and how bold, italic, underline
and strikethrough look. Tokens live in [`theme.rs`](../../src/tui/theme.rs).

## Tokens

Text roles, with WCAG contrast on the reference dark background `#17161F`.
Tests hold every text role at 4.5 or more and `FAINT` at 3 or more from
`#0C0C0C` to `#1E1E1E`; lighter dark themes such as One Half Dark (`#282C34`)
bring `DIM` and `FAINT` below those floors.

| Role | Value | Contrast | Use |
| --- | --- | --- | --- |
| `STRONG` | `#E2E8F0` | 14.5 | Requests, drafts, the product name, titles, labels, option names, keycap keys. |
| `LIT` | `#E2D9FC` | 13.3 | The caret's blinking `_`, chips, background badge names, command names in Work chapters, `new output`. |
| `BODY` | `#D4D4D4` | 12.1 | Assistant prose, tool targets, diagnostics. |
| `CODE_TEXT` | `#C9D1D9` | 11.6 | Default code, inline code, command output, diff context. |
| `GOOD` | `#9ECE6A` | 9.8 | The `Done` and `✓` marks, added lines, `☑`, code strings. |
| `ACCENT` | `#C3B1FA` | 9.4 | The `>_` mark, `›` and `●` markers, composer rules, matches, level 1 headings, links. |
| `KEYWORD` | `#E0A6F2` | 9.3 | Code keywords. |
| `WARNING` | `#E0AF68` | 9.0 | Retry, stopped and warning glyphs, `paused`, code numbers, the gauge near the limit. |
| `BAD` | `#EF8B8B` | 7.5 | Failures, removed lines, destructive questions. |
| `MUTED` | `#9AA4B2` | 7.1 | Secondary facts, key labels, link hosts, quotes, list markers, the footer chip. |
| `DIM` | `#8E8AAE` | 5.5 | Status-row facts, the footer path, tree gutters, group headings, line numbers, struck text. |
| `FAINT` | `#6B6796` | 3.4 | Marks only, never text: rules while working, table borders, tracks. |

The activity shimmer uses `S0` `#A99BE6`, `S1` `#D9CBFB` and `S2` `#FBF8FF`
over `S_BASE` `#6E64A0`. Tool badge labels use `BADGE_TEXT` `#F2F5FC`; the
software caret is `#1A1B26` on `#C0CAF5`.

Surfaces, 1.2–1.6 contrast steps above a dark terminal background:

| Surface | Value | Use |
| --- | --- | --- |
| `BASE` | terminal default | Every cell without its own surface; never painted. |
| `USER_SURFACE` | `#29283F` | Sent requests. |
| `PILL_SURFACE` | `#27263B` | The status-row pill and the footer model chip. |
| `SELECTED_SURFACE` | `#383656` | The focused option row and keycaps. |
| `CODE_SURFACE` | `#20252C` | Code cards, inline code, process output. |
| `DIFF_ADDED_SURFACE` / `DIFF_REMOVED_SURFACE` | `#1E2E23` / `#312026` | Added and removed diff rows. |
| `DIFF_ADDED_STRONG` / `DIFF_REMOVED_STRONG` | `#2C4A33` / `#523039` | Paired changed fragments. |

Tool badges carry the family colour when done and the state otherwise:

| Badge surface | Value | Use |
| --- | --- | --- |
| `BADGE_READ` | `#4C4AB2` | Read, List, Search, Map, Skill, Web, Browser. |
| `BADGE_EDIT` | `#7A3F9E` | Update, Write. |
| `BADGE_SHELL` | `#3D58A3` | Bash, Exec, Process. |
| `BADGE_OTHER` | `#4C4AB2` | Checkpoint and any other tool. |
| `BADGE_IDLE` | `#34324F` | Running and background badges, `WORK`, attachment and ` current ` chips. |
| `BADGE_BAD` | `#7A2E3A` | Failed calls. |
| `BADGE_WARN` | `#6B5222` | Stopped or timed-out calls. |

## Screen and grid

From the top: the header, the conversation, one gap row, then the dock. The
header is the start of the conversation and scrolls away with it; the dock is
anchored to the bottom. Markers (`›`, `●`, `✕`) and badges sit at column 0,
text starts at column 2, and right-hand facts end two cells before the edge.
One blank row separates conversation blocks. Chips, badges, pills and keycaps
have one cell of padding.

The dock holds, top to bottom: queued drafts, the status row, the composer
(top rule, draft, bottom rule) and the footer, key hints or an open panel. See
[Composer and controls](COMPOSER.md).

The header reads `>_ jecode <version>`, with `>_` bold `ACCENT`, `jecode`
bold `STRONG` and the version `MUTED`, shortened from the right. The model,
effort and folder sit in the footer, so each appears once on screen.
Until the conversation shows its first item, the space below lists
`Recent in this folder` in `DIM`: up to three other conversations of this
folder with compact ages such as `2h`, then a ` /resume ` keycap for all
conversations. Deleting one through `/resume` updates the list at once. With
none saved, on a first run or after deleting the last, one key row shows instead:
`/ commands · Alt+V image · Ctrl+O expand · Ctrl+Q quit`. Both key rows drop
keys from the right when narrow.

## Status row and motion

The status row sits above the composer and is always one row. Every state has
the same shape: a pill on `PILL_SURFACE` (` glyph label `), `DIM` facts joined
by ` · ` and dropped from the right, then an optional keycap ending two cells
from the edge, which drops after the facts.

| State | Glyph | Label | Facts |
| --- | --- | --- | --- |
| Idle | — | empty row | — |
| Working | blinking `>_` | activity, shimmering | `Esc interrupt · turn time · ↓ tokens` |
| Retry | blinking `>_` in `WARNING` | `Retrying in 4s` in `WARNING` | `Esc interrupt · turn time · attempt N · reason` |
| Done | static `>_` in `GOOD` | `Done` | `turn time · N tools · ↓ tokens`, then the serving provider when reported |
| Stopped | `■` in `WARNING` | `Stopped` | `time · your next message continues from here` |
| Failed | `✕` in `BAD` | `Failed` | `time · prompt restored` and a fixing keycap |
| Result | `✓` in `GOOD` | subject, such as `Copied` | detail; a setting reads `old → new` with the new value `STRONG` |
| Warning | `▲` in `WARNING` | subject | detail and a fixing keycap |
| Error | `✕` in `BAD` | subject | detail and a fixing keycap |

`Esc interrupt` appears only while Esc would stop the turn. The Working label
is the running call's `description` when given, else a verb and a short
target: `Reading`, `Editing`, `Writing` a file; `Running` a command;
`Starting` an `exec` program; `Fetching` or `Browsing` a host;
`Loading skill`; `Saving a checkpoint`. Between calls it names the phase, such
as `Thinking`, `Working` or `Waiting for your edit`; `Stopping` takes priority
after cancellation. Compaction and attachment progress take the label while
they run. Composer rules turn `FAINT` while a turn works.

One motion frame is 100 ms. The caret keeps `>` in `ACCENT` and blinks `_` in
`LIT`, 400 ms on and 400 ms off. One shimmer band, `S0 S1 S2 S1 S0` moving one
cell per frame over `S_BASE`, runs through the status label, running badges
and running Work chapters.

## Conversation

A sent request is a full-row `USER_SURFACE` block with one padding row above
and below: `› ` in bold `ACCENT` at column 0 and the text in `STRONG` at
column 2, wrapped rows hanging under it. Request text stays plain, apart from
attachment labels, which become ` 1 Image ` or ` 2 wake.log ` chips in `LIT`
on `BADGE_IDLE`.

An answer opens with `●` in `ACCENT`, prose at column 2; see
[Markdown responses](MARKDOWN.md). Reasoning stays hidden. A failed turn keeps
its diagnostic as the turn's last row: `✕` in bold `BAD`, then the text in
`BODY`.

## Tool rows

Each call opens with a badge, ` NAME ` in bold at natural width, then the
target one space after in `BODY`. There is no separate status dot: a finished
call keeps its family colour, a running one shimmers on `BADGE_IDLE`, a
background one shows a `LIT` name on `BADGE_IDLE`, a failed one turns
`BADGE_BAD` and a stopped one `BADGE_WARN`. The names are READ, SKILL (a
`read` of a skill's `SKILL.md`), UPDATE (the `edit` tool), WRITE, BASH, EXEC,
PROCESS, CHECKPOINT, WEB, BROWSER, LIST, SEARCH and MAP. List, Search and Map use the
read-family Violet surface; an optional description keeps the concrete target
on the following tree row.

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
| Running | The elapsed time, after `Running` for routine calls, and any note, such as the [checkpoint](../usage/CHECKPOINTS.md) `checkpoint rc-ID` recorded before a Bash call. |
| Read | `9 lines · from line 33 of 212`, or `9 lines · from line 33 · more from 42` when the total is unknown. Narrow rows drop the start clause; split lines keep `continues at byte N`. Skill reads `Loaded skill · …`. |
| List, Search | Entry or matching-line counts, searched files, errors and `more from N` when paged. Expanded cards show structured paths, source context and exclusions. |
| Map | Overview counts entries, files, notes, errors and continuation; inventory truncation reads `partial`. Expanded cards show areas, landmarks, symbols, notes, source references, freshness and source inspection errors. Remember and forget show concise note receipts. |
| Update, existing-file Write | `+N −M lines` in `GOOD` and `BAD`, or explicit text for no-op edits, final newlines and line endings. |
| New Write | `New file · N lines`, then the first six numbered source rows. |
| Checkpoint | One quiet result with the hidden row count. |
| Bash, Exec, Process | The last output line with the duration from one second, `Done in 1.2s`, or `Done`; `+N lines · no checkpoint` follows in `DIM`. |
| Failed command | `Exit N · first line` in `BAD` on at most two rows, a muted position, up to two more `BAD` stderr rows. |
| Stopped command | `Cancelled after 6.0s` or `Timed out after …`, `· partial output` when some was captured, and the hidden line count. |
| Background | `p3 running · latest line`; Process reports `stopping`, `stopped` or `status unknown`. A process list shows one count. |
| Web | The page title, size and host. |
| Browser | Where it landed or what it saved, such as `Screenshot 1280×720 · path`. |
| Error | The decoded message in `BAD`; a missing path reads `Not found`. |

Compact cards skip blank output lines and fold the hidden count into the
result. Capture limits stay visible as muted notes, such as `stdout trimmed to
the last 64 KiB of 89 KiB · full output saved`. Checkpoint saves are quiet:
partial coverage shows a warning; missing coverage joins a routine call's
result, while file cards keep their warning.

`Ctrl+O` expands every call of the latest finished turn into its full card:
the full command and exact output, numbered source for Read, all diff hunks,
and `exit N · duration` for commands. Stdout uses `CODE_TEXT`, successful
stderr `MUTED`, and stream labels `DIM` when both streams exist. An expanded
image Read shows an `Opened image` fact with dimensions and bytes; Web and
Browser keep their raw results. Expansion adds a call's `Checkpoint rc-ID`
reference. The last card of that turn with hidden content carries a
` Ctrl+O ` keycap with `expand` in `DIM` at the right of the row reporting it;
expanded, the turn's first card names `collapse` on its title. Tool cards keep the result reported when the call returned;
`/terminal` shows the live process list.

### Work chapters

Two or more adjacent Read, List, Search, Map overview, Bash, Exec, Process, Web and Browser calls share a
Work chapter when their normalized `work` labels and execution turn match.
Skill reads, Map remember/forget, assistant text and other tools break the chapter; a single call
keeps its normal card. A missing label displays as **Activity**; the label is
collapsed to one line and ellipsized to 60 cells for display only.

The title is a ` WORK ` badge, the label in bold `STRONG`, `N calls` in `DIM`
and `· N failed` in `BAD`. The badge carries the chapter's state: `STRONG` on
`BADGE_IDLE` when done, shimmering while running, red or amber on failure or
stop. When the chapter carries the turn's ` Ctrl+O ` keycap, it sits at the
title's right edge and the title clips before it.

One tree runs from the title to the chapter's last row. The first row counts
successful work: unique files read, listings, searches, maps, other reads, web calls, screenshots,
snapshots, browser actions, commands and process calls, plus running,
background and stopped calls; it reads `0 completed` when it has nothing else.
Each command keeps one row with its family-coloured name and description or
command, with running time or a failed `exit N` inline. Other failed or
unusual calls keep a row and bounded errors. The latest ordinary Read, Web or
Browser call stays at the foot, with its time frozen when done. A chapter
that replaced a single card keeps at least that card's height. Expanded, the
title's tree closes on its facts, every call follows as its full card, and the
foot row is dropped.

### Diffs

Update and existing-file Write show contextual diffs with three surrounding
lines: original coordinates on removals, resulting coordinates on additions
and context. Rows run from column 2 to the edge; added and removed rows are
tinted, context rows stay on the base in `CODE_TEXT`. Line numbers are `DIM`,
marks `GOOD` `+` and `BAD` `−`. Paired changed fragments alone use bold
`GOOD`/`BAD` on the stronger surface; other changed code keeps its syntax
colours; fragments snap to whole words, and in markup to whole tags and
entities. Wrapped code breaks at token boundaries, as in
[Markdown](MARKDOWN.md) code cards, and keeps an empty number gutter.
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

Panels open below the composer in place of the footer. The title rides the
composer's bottom rule as `Parent › Title · context`: parent and context
`MUTED`, title bold `STRONG`, an aside such as `3 of 12 models` on the right.
Typed input stays in the composer, whose top rule names the field.

Option rows put `›` in `ACCENT` at column 0 on the selected row, which takes
`SELECTED_SURFACE` across the whole row. Names are `STRONG` at column 2 with
matched characters in bold `ACCENT`; a `MUTED` fact column follows at the
widest name plus two, at most three fifths of the width. Badges and ages end
on the right; the current item carries a ` current ` chip in `LIT` on
`BADGE_IDLE`. Group headings are `DIM`. A `┃` thumb in `MUTED` on a `│` track
in `FAINT` takes the last column when the list overflows.

Tabs show names in `MUTED`, the active one bold `STRONG` over an `ACCENT` `━`
on a `FAINT` divider, with an optional mode word on the right. Process output
sits on `CODE_SURFACE` from column 2 to the edge.

Keycaps are ` key ` in `STRONG` on `SELECTED_SURFACE` with the label in
`MUTED`, three cells apart. A confirmation replaces the keys with its question
in `BAD`, and the marked row turns `BAD` until confirmed or cancelled.

## Reading back and exit

While you read older output, the gap row above the dock shows
`↓ N rows below`, with `· new output` in `LIT` once output arrived after you
scrolled away; the arrow is `ACCENT` then, `DIM` otherwise. A keycap
`Esc back to bottom` ends two cells from the edge, or `Alt+End` while a panel
or a draft edit holds Esc; it drops first when narrow. While a resize reflows
the conversation, the gap row may show `Reflowing conversation…`.

After the alternate screen closes, the normal screen keeps the terminal's own
colours. The exit summary uses only ANSI magenta for `>_`, bold for `jecode`,
SGR dim for the facts and the default foreground for the resume command; see
[Scrolling and resize](RESIZE.md#entering-and-leaving).
