# Tool activity

Every tool call the agent makes appears in the conversation as a compact card,
so you can follow the work without reading full output. This page explains
what the cards show. The tools themselves are described in
[Tools](TOOLS.md).

## Tool cards

A card starts with a badge naming the tool (READ, UPDATE, SAVE, BASH, EXEC,
PROC, LIST, SEARCH, HTTP, BROWSE, MCP CALL, LSP CALL, SKILLS or AGENTS), then
its target, such as a file path or a command. Every badge fills eight cells with
its name centred and equal padding on both sides, so targets line up from
column 9. These are display labels; tool identifiers and commands keep their
existing names. The badge color shows the
state: the tool family's color when finished (MCP, LSP and tools without a
family use a quiet grey), a moving shimmer while running, red when
the call failed and amber when it was stopped or timed out. A result line hangs
below the title:

| Call | Result |
| --- | --- |
| Read | Lines shown and where, such as `9 lines · from line 33 of 212`. A skill read shows `Loaded skill`. |
| Read (several paths) | `3 files · 120 lines`, adding `N not read` when a path failed and `N saved` when results went to a result file, counted within the batch total, then each file under its path and a last row naming the result file. |
| Read (image or PDF) | `Opened image · 800×600 · 12 KiB`, or `Opened PDF · 4 pages · 1.2 MiB`. |
| List, Search | Entry or match counts, files searched, errors, and `more from N` when there is another page. |
| Update | `+N −M lines`, followed by the diff. Used for `edit` and for a `write` that replaced an existing file. |
| Update (several files) | One card naming every path, with `3 files · +9 −1 lines` and `N failed` in red. The first three diffs stay open; later files keep their heading. |
| Save (new file) | `New file · N lines` and the first six lines. |
| Bash, Exec, Proc | The last output line and the duration, such as `Done in 1.2s`. |
| Failed command | `Exit N` with the error and up to two lines of stderr, in red. |
| Stopped command | `Cancelled after 6.0s` or `Timed out after …`, noting partial output. |
| Background command | `p3 running` and the latest output line; a `&` follows the title while it runs. |
| HTTP | The page title, size and host. |
| Browse | Where it navigated or what it saved, such as `Screenshot 1280×720`. |
| Any error | The error message, such as `Not found`. |

A `steps` call shows each step as its own card, in order, live and after resume.
Only a refused call has a `SEQUENCE` card, with the tools it named and the reason.

`MCP CALL` cards use the quiet badge surface and identify the connection and remote
operation, such as `GitHub › create_issue`. Arguments use the same `│` row,
omitted when a call has none, and results the same `└` row as other tools.
Ctrl+O shows the full arguments and result; each MCP call keeps its own row
inside a Work chapter. Cancellation before dispatch uses the stopped badge
and `Cancelled` in amber: the call was not sent. An uncertain remote outcome uses amber and
**Outcome unknown**: the service may already have acted. Check it before
retrying.

`LSP CALL` cards use the same quiet badge and `server › action` target. Errors and
warnings found by diagnostics are successful analysis; their counts use the
existing error/warning colors. Freshness states and document coverage stay
visible, and a current empty report applies to that file only. Timeout and
cancellation use stopped; crashes and missing executables use failed. Each
call keeps one row in a mixed Work chapter; expanding preserves every call.

When the agent supplies a `description` for a command, the card shows it as the
title and the command beneath it (`$ command` for Bash, `> program args` for
Exec). Blank output lines are skipped in compact cards. If output was trimmed,
a note such as `stdout trimmed to the last 64 KiB of 89 KiB · full output saved`
says so.

Cards show the result as it was when the call returned. For a process still
running, use `/process`. When a process that no call is waiting for ends, a
`PROC` row reports its command and outcome at that point in the conversation;
Ctrl+O opens the end of its output.

## Work chapters

Two or more consecutive routine calls (Read, List, Search, Bash, Exec,
Process, Web, Browser, MCP, LSP, collaboration events and agent list,
inspect, wait and diff) with the same `work` label in the same turn are
grouped into a **Work chapter**. The agent chooses the label; calls without one
are grouped as **Activity**. Assistant text, a different label, a skill read or
any other tool ends the chapter, and a single call keeps its own card.

A chapter's title shows a WORK badge, the label, the number of calls and any
failures. Below it:

- one row counts what was done: files read, searches, commands, web calls and so
  on, plus running, background and stopped calls;
- every command keeps its own row, with its running time or a failed exit code;
- failed or unusual calls keep a row with their error;
- up to four of the latest completed reads, listings, searches, web and
  browser calls stay at the bottom.

When several calls run in parallel, each running call has its own row, and the
status row reads `N tools running`.

## Diffs

Successful edits show a diff with three lines of context: line numbers, `+` and
`−` marks, and red or green tinted rows. Within a changed line, the changed words
are highlighted more strongly, while the rest keeps its syntax colors. Long
lines are cut short in compact cards so the change stays visible.

Short diffs stay open. Longer ones show the first hunk, summarize long runs as
`… N added lines` and report the rest, such as `+1 hunk · 6 rows`. Removing a
final newline, changing line endings and edits that change nothing are stated in
words. Failed calls never show a diff.

Diffs are saved with the conversation and replayed on resume without rereading
the file. Very large changes (over 1 MiB or 20,000 lines, or too costly to
align) show a plain preview instead. These limits affect only the display, never
the edit.

## Expand with Ctrl+O

When the agent is idle and no panel is open, Ctrl+O expands every tool call of
the latest finished turn into its full card: the complete command and output,
numbered file content, all diff hunks, exit codes and durations. Press Ctrl+O
again to return to compact cards. The view keeps its reading position, and a
short notice confirms how many tools were expanded.

Ctrl+O applies only to the latest finished turn and does nothing if that turn
had no tool calls. The last card with hidden content shows a `Ctrl+O expand`
key, as text two cells from the edge of its row. Sending a new message
collapses the expanded turn, and resumed
conversations start collapsed.

## Collaborators

Collaboration cards use the fixed `AGENTS` badge. Individual events put the
display number, such as `#3`, outside the colored block, so growing numbers
never resize the badge. Assignments sent and received, returns, reviews, stops,
closures and unknown outcomes join the Work chapter of the agent call that
produced them, and appear in the same place live and after resume. Failures are
red, unknown outcomes amber. Plain member rows inside a chapter keep the
`AGENT N` identity. Ctrl+O shows each event's full receipt:
instructions, workspace, changed files with `+` and `−` line counts, report and
review state. For a collaborator's current state, use `/agents`; see
[Collaborating agents](COLLABORATION.md#watch-in-the-tui).

## Other rows

- **TODO** marks the creation of a plan, with its title and task count; see
  [Questions and plans](QUESTIONS_AND_PLANS.md#plans).
- **ASK USER** shows the answers you gave to the agent's questions.
- Answers start with `●`, and Markdown in them is formatted; see
  [Markdown display](../reference/MARKDOWN.md). Reasoning is not shown.
