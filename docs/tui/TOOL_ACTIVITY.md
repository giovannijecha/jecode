# Tool activity

Every tool call the agent makes appears in the conversation as a compact card,
so you can follow the work without reading full output. This page explains
what the cards show. The tools themselves are described in
[Tools](../usage/TOOLS.md).

## Tool cards

A card starts with a badge naming the tool (READ, UPDATE, WRITE, BASH, EXEC,
PROCESS, LIST, SEARCH, MAP, CHECKPOINT, WEB, BROWSER, SKILL or AGENTS), then
its target, such as a file path or a command. The badge color shows the state:
the tool family's color when finished, a moving shimmer while running, red when
the call failed and amber when it was stopped or timed out. A result line hangs
below the title:

| Call | Result |
| --- | --- |
| Read | Lines shown and where, such as `9 lines · from line 33 of 212`. A skill read shows `Loaded skill`. |
| List, Search | Entry or match counts, files searched, errors, and `more from N` when there is another page. |
| Map | Counts of entries, files and notes; `partial` when the survey was cut short. |
| Update | `+N −M lines`, followed by the diff. Used for `edit` and for a `write` that replaced an existing file. |
| Write (new file) | `New file · N lines` and the first six lines. |
| Bash, Exec, Process | The last output line and the duration, such as `Done in 1.2s`. |
| Failed command | `Exit N` with the error and up to two lines of stderr, in red. |
| Stopped command | `Cancelled after 6.0s` or `Timed out after …`, noting partial output. |
| Background command | `p3 running` and the latest output line; a `&` follows the title while it runs. |
| Web | The page title, size and host. |
| Browser | Where it navigated or what it saved, such as `Screenshot 1280×720`. |
| Checkpoint | A short result; Ctrl+O shows the rest. |
| Any error | The error message, such as `Not found`. |

When the agent supplies a `description` for a command, the card shows it as the
title and the command beneath it (`$ command` for Bash, `> program args` for
Exec). Blank output lines are skipped in compact cards. If output was trimmed,
a note such as `stdout trimmed to the last 64 KiB of 89 KiB · full output saved`
says so. A checkpoint saved before a command appears as a quiet note; partial
or failed checkpoint coverage shows a warning.

Cards show the result as it was when the call returned. For the current state of
a background process, use `/terminal`.

## Work chapters

Two or more consecutive routine calls (Read, List, Search, Map overview, Bash,
Exec, Process, Web and Browser) with the same `work` label in the same turn are
grouped into a **Work chapter**. The agent chooses the label; calls without one
are grouped as **Activity**. Assistant text, a different label, a skill read or
any other tool ends the chapter, and a single call keeps its own card.

A chapter's title shows a `WORK` badge, the label, the number of calls and any
failures. Below it:

- one row counts what was done: files read, searches, commands, web calls and so
  on, plus running, background and stopped calls;
- every command keeps its own row, with its running time or a failed exit code;
- failed or unusual calls keep a row with their error;
- up to four of the latest completed reads, listings, searches, maps, web and
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
numbered file content, all diff hunks, exit codes and durations, and each
call's checkpoint reference. Press Ctrl+O again to return to compact cards. The
view keeps its reading position, and a short notice confirms how many tools
were expanded.

Ctrl+O applies only to the latest finished turn and does nothing if that turn
had no tool calls. The last card with hidden content shows a `Ctrl+O expand`
key. Sending a new message collapses the expanded turn, and resumed
conversations start collapsed.

## Collaborators

Collaboration events appear as rows: `AGENTS` when collaborators are created,
then `AGENT N` rows for assignments sent and received, returns, reviews, stops,
closures and unknown outcomes. They join the Work chapter of the agent call that
produced them, and appear in the same place live and after resume. Failures are
red, unknown outcomes amber. Ctrl+O shows each event's full receipt:
instructions, workspace, changed files with `+` and `−` line counts, report and
review state. For a collaborator's current state, use `/agents`; see
[Collaborating agents](../usage/COLLABORATION.md#watch-in-the-tui).

## Other rows

- **TODO** marks the creation of a plan, with its title and task count; see
  [Questions and plans](../usage/QUESTIONS_AND_PLANS.md#plans).
- **ASK USER** shows the answers you gave to the agent's questions.
- Answers start with `●`, and Markdown in them is formatted; see
  [Markdown display](MARKDOWN.md). Reasoning is not shown.
