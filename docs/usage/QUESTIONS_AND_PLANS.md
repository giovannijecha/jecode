# Questions and plans

Two tools help the main agent work with you: `ask_user` stops to ask you
something and waits for the answer, and `todo_list` keeps a plan for the
conversation that you can check at any time.

## Answer questions

When the agent asks, the questions replace the composer under a
`Waiting for you` marker, and an open menu or panel closes so they show at once.
One request holds up to four questions, answered one at a time and sent
together. A question can offer up to six options, single or multiple choice,
and always ends with **Other**, where you type your own answer. Nothing is
preselected; an option marked as recommended is only a suggestion.

| Key | Action |
| --- | --- |
| Up/Down | Move between options without choosing. |
| `1`–`N` | Choose an option directly when nothing is focused; the last number focuses **Other**. |
| Space | Select the highlighted option; for multiple choice, toggle it. |
| Typing | Fill **Other**; Ctrl+J adds a line. |
| Enter | Choose the highlighted option and move on; on the last question, send all answers. |
| Tab, Shift+Tab, Left/Right | Switch questions (Left/Right only outside typed text). |
| Esc | Return to the main conversation, leaving the questions pending. `/ask` reopens them. |
| Ctrl+D | Dismiss the questions; the agent learns that you gave no answer. |
| Ctrl+C | Stop the turn. |

If the questions arrive while you are typing a draft, nothing is focused and
Enter does nothing; your keys keep editing the draft until you pause or press
an arrow key or Tab. Answers in progress and your draft keep separate text.
While the main agent waits, collaborators and background processes keep
working, and `/agents` and `/todo` remain available.

After you answer, the conversation shows one `ASK USER` row with your answers.
Pending questions and answers in progress survive `/resume`; tool calls the
agent had planned after the question are recorded as not executed rather than
replayed.

In `--plain`, questions are asked one by one: type an option's number, a list
such as `1,3` for multiple choice, or your own words; `/dismiss` skips. Each
answer is saved before the next question. Single prompts and pipes cannot ask,
and collaborators never ask you directly.

### Question format

```json
{
  "questions": [
    {
      "id": "format",
      "title": "Format",
      "question": "Which format should the export use?",
      "context": "The current exporter produces one file.",
      "options": [
        {"id": "csv", "label": "CSV", "description": "Open in a spreadsheet", "recommended": true},
        {"id": "json", "label": "JSON", "description": "Keep nested fields"}
      ]
    },
    {
      "id": "fields",
      "title": "Fields",
      "question": "Which fields should be included?",
      "multiple": true,
      "options": [
        {"id": "dates", "label": "Dates"},
        {"id": "owners", "label": "Owners"}
      ]
    }
  ]
}
```

The result tells the model whether you answered, dismissed, stopped the turn or
could not be asked. An answer lists, for each question ID, the chosen option IDs
and any text you typed.

## Plans

The plan belongs to the model; to change it, ask in the conversation. When the
model creates a plan, the conversation shows one `TODO` row with its title and
task count. Later updates add no rows.

`/todo` opens the current plan below the composer and updates live: the title,
a progress bar, `N of M done`, and each task with its number, owner and state
(`done`, `in progress`, `waiting`, `awaiting review`, `failed` or `cancelled`; a
task not started shows no state). Arrow keys scroll; Esc closes. It makes no
model request. In `--plain`, `/todo` prints the same information.

A task can name the collaborator working on it, shown as an `AGENT N` badge.
When that collaborator returns, the task shows `awaiting review` (or `failed`)
until the main agent updates it; a collaborator's return never completes a task
by itself. Collaborators keep their own plans.

The plan survives resume, compaction and model changes, and is included in
exports. `/new` starts without one. It records the model's account of progress;
Jecode does not enforce it.

### Plan format

`{"action": "get"}` returns the current plan (revision 0 with no tasks if none
exists). `set` replaces the whole plan:

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

Statuses are `pending`, `in_progress`, `waiting`, `done` and `cancelled`. `note`
(visible only to the model) and `agent` (a collaborator such as `a1`) are
optional. A plan holds up to 40 tasks. Each successful update returns the full
plan with a new revision number; an invalid update leaves the previous plan
unchanged. A `set` after every task is done or cancelled starts a new plan.
