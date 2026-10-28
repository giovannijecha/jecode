# Questions and plans

`ask_user` lets the main agent wait for a concrete answer. `todo_list` keeps a
plan for the current conversation. Both appear as chronological tool cards and
run in source order, even if a call includes `parallel: true`.

## Answer a question

In an interactive terminal, a question opens below the composer. It can include
context and up to six choices. Free text is always available, including when
choices are present. A recommendation is a suggestion; it is never submitted
automatically. Submit a selected choice, text, or both explicitly.
Tab switches between text and choices, arrows move through choices, and Enter
submits. Ctrl+D explicitly dismisses the question. Ctrl+C stops the active turn.

Esc returns to the main chat while leaving the question pending. `/ask` reopens
it. The answer draft and the main draft keep separate text and carets. Dismissing
a question is a separate explicit action and tells the model that no answer was
supplied. The main agent waits; existing collaborators and processes can keep
working, and `/agents` and `/todo` remain available for inspection.

The tool accepts:

```json
{
  "title": "Export format",
  "question": "Which format should the export use?",
  "context": "The current exporter produces one file.",
  "options": [
    {"id": "csv", "label": "CSV", "description": "Open in a spreadsheet", "recommended": true},
    {"id": "json", "label": "JSON", "description": "Keep nested fields"}
  ]
}
```

The result distinguishes an answer from dismissal, cancellation or unavailable
interactive input. Answered results include the selected choice ID, when any,
and the entered text. Single-prompt and piped invocations return unavailable
without blocking. Interactive `--plain` chat accepts answers through stdin.
Collaborators do not receive this tool; they report questions to the main agent.

Pending questions and answer drafts survive `/resume`. A saved answer remains
associated with its original tool call, including if Jecode closes before the
model continues. Restoring a question does not replay earlier file or command
effects. Old ASK cards retain the answer that was actually submitted.
Closing the input surface preserves a pending question; stopping the turn
explicitly records cancellation instead.

## Inspect a plan

The plan belongs to the model; ask for changes in the main conversation. The
conversation shows only the call that creates a plan: a `TODO` row with the plan
title and its task count, which stays as written. Reads, later updates, replans
and the end of the plan add no row. A `set` with no plan yet, or after every
task of the previous plan is done or cancelled, creates a new plan and a new row.

`/todo` opens the current plan below the composer and updates live. The rule
shows the plan title, one progress cell per counted task and `N of M done`.
Every task appears in plan order with its number, mark, title, owner and state:
`done`, `in progress`, `waiting`, `awaiting review`, `failed` or `cancelled`;
a task not started has no state word. Cancelled tasks keep their place without
a number. Narrow terminals drop the numbers and shorten `awaiting review` to
`review`. The menu has nothing to select or open: arrows scroll a long plan and
Esc closes it. It does not make a model request. In interactive `--plain`,
`/todo` prints the same information as text.

`todo_list` with `action: "get"` returns the current plan, or revision zero with
an empty task list when no plan exists. With `action: "set"`, it replaces the
complete plan and assigns the next revision:

```json
{
  "action": "set",
  "title": "Add the export",
  "tasks": [
    {"id": "inspect", "title": "Inspect the existing format", "status": "done"},
    {"id": "implement", "title": "Implement the exporter", "status": "in_progress"},
    {"id": "verify", "title": "Verify the exported file", "status": "pending", "note": "Check an empty input too"}
  ]
}
```

Task IDs remain stable when editing a task. Supported statuses are `pending`,
`in_progress`, `waiting`, `done` and `cancelled`. `note` and `agent` are optional
text; notes are for the model and `/todo` does not show them. A plan may contain
up to 40 tasks. An empty `tasks` array clears its activities while retaining the
new revision in history.

`agent` names the collaborator working on a task, such as `a1`, and `/todo`
shows it as an `AGENT N` badge with its lifecycle. While the main agent has not
marked the task done, cancelled or waiting, a returned collaborator shows the
task as `awaiting review` and a failed one as `failed`. Only the main agent marks
its plan; a collaborator's return never completes a task, and collaborators keep
their own plans.

Every successful update returns the full plan with its revision; `/todo` shows
the latest successful update. Invalid updates leave the previous plan intact.
There is no permanent Todo footer.

The plan survives resume, compaction and model changes, and is included in the
conversation export. `/new` starts without a plan. The plan records the model's
account of progress; it does not enforce a completion gate.
