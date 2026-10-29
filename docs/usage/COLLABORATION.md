# Collaborating agents

The main agent can use `agent` to delegate focused assignments, revise them while
work is in progress, inspect evidence and review the result. You continue talking
to the main agent. `/agents` provides read-only inspection without making a model
request or marking a result reviewed. Review belongs to the main agent; you do
not have a review queue. The panel lists open assignments, including returned
work still being assessed by the main agent.

Each collaborator starts with its own conversation and the main agent's current
model, effort and routing. It receives the assignment, supplied context, scoped
project instructions, personal skills and access to shared project-map notes.
The whole parent conversation is not copied. Follow-up assignments retain the
collaborator's conversation. Changes to the main agent's model settings do not
reconfigure existing workers.

Collaborators have the same file, command, process, checkpoint, web, browser and
map tools. Their tool catalog excludes `agent`; runtime dispatch also rejects
delegation from a collaborator. Child processes, browser handles, scratch output
and session histories belong to the individual worker.

## Assign and coordinate

The main model supplies an objective, file or component scope, relevant source
references, expected deliverable and checks in `instructions` and `context`.
For example, an `agent` call can create independent analysis and implementation
assignments together:

```json
{
  "action": "create",
  "tasks": [
    {
      "title": "Inspect search behavior",
      "instructions": "Read the search implementation and tests. Report edge cases with source references; do not edit files.",
      "context": "The current task concerns empty queries.",
      "workspace": "shared"
    },
    {
      "title": "Implement empty-query behavior",
      "instructions": "Update the search component and focused tests. Report changes, executed checks and unresolved issues.",
      "context": "An empty query should preserve the original result order.",
      "workspace": "isolated"
    }
  ]
}
```

`create` accepts one to eight assignments per call. It returns stable IDs such as
`a1`; the interface displays `AGENT 1`. IDs are never reused in a conversation.
The displayed number is the lowest one no open collaborator holds, so a single
assignment created after the previous one closed is `AGENT 1` again, while
collaborators open at the same time keep distinct numbers. Each control call is serialized with other
main-agent tools, even with `parallel:true`. The workers continue independently.

| Action | Required fields and behavior |
| --- | --- |
| `create` | `tasks`, each with `title`, `instructions`, `workspace`; optional focused `context`. |
| `update` | `id`, `instructions`; optional `context`. Sends a new version during work or starts follow-up work on an available returned worker. |
| `list` | Returns open assignments in the current conversation. |
| `inspect` | `id`. Returns activity, assignment versions and report evidence; does not review it. |
| `wait` | Optional `id` and `timeout_ms` (0–60000). Waits for selected running work, remaining cancellable. |
| `stop` | `id`. Requests cancellation; a request is distinct from a confirmed stop. |
| `review` | `id`, `report_version`, `note`. Records the main agent's assessment of the latest returned assignment. |
| `diff` | `id`. Compares an isolated workspace against its starting state and the current main project. Optional `path`, byte `offset` and `limit` page before/after content. |
| `integrate` | `id`, explicit `paths`, `expected_review_token` from `diff`. Applies selected changes with conflict checks. |
| `close` | `id`. Releases the worker and its tool resources, removes its owned isolated copy and removes the assignment from the active list. A chronological `closed` receipt retains the final record. Close after preserving needed work. |

Supply the complete revised assignment in `update`. An update being sent does
not mean it has been received. Running workers consume
updates at model boundaries; a tool already executing may finish first. Reports
identify their received assignment version. Older reports remain evidence of
older work and cannot review a newer active assignment.

Returned work still needs the main agent's inspection, correction and verification.
`review` records that assessment; it does not run tests or imply integration.
After any needed correction or integration, the main agent calls `close` to
finish the assignment. Reading or summarizing a report does not perform either
action. A closed collaborator cannot receive more work; create a new assignment
when needed; its ID is not reused, though its displayed number may be.
The TUI receives return events while the main agent is idle and resumes the main
model to examine new work without inserting a user message. Queued user input
takes priority. Explicitly stopping the main turn suppresses this automatic
continuation until another user turn starts. Historical receipts restored from
disk do not trigger a model request.

## Choose the working space

`shared` uses the current project directly. It suits analysis and coordinated
changes, but workers can affect the same files and external state. The main agent
must assign compatible scopes and inspect the combined result.

`isolated` creates a detached local Git worktree outside the project. It includes
the current contents of tracked files, tracked deletions and non-ignored untracked
files, including uncommitted changes. Scoped local `AGENTS.md` files are included.
Ignored build output and other ignored local data are not generally copied. The
starting files are retained separately for comparison; no commit or branch is
created. Native Git must be available, and the project must be a Git repository
root with an existing `HEAD`.

Very deep Windows paths can still exceed Git's own directory limits. Jecode
reports the native failure and any incomplete cleanup; it does not start a
worker with a partially prepared copy. Shared work remains available when an
isolated copy cannot be prepared.

This is file isolation, not an operating-system sandbox. Commands and browser
operations retain ordinary system access. A worktree shares its repository's Git
administration; coordinate repository operations and other external effects.

`diff` returns changed paths, baseline/child/current fingerprints, bounded change
evidence and a review token. For longer files, request `diff` with `path`; byte
pages default to 16384 and accept at most 65536 bytes. Binary or unsupported text
is identified rather than presented as a complete text diff.

`integrate` rechecks the token and each selected path. Main-project changes since
the starting state are conflicts unless they already match the child. The result
reports applied, already-current, conflicting and failed paths. Integration is
per file, not an atomic multi-file transaction. Ordinary file additions, edits
and deletions are supported; link and directory-type changes require explicit
coordinator handling. Integration captures the selected main-project paths in a
checkpoint and includes its receipt in the result. Each worker also has its own
checkpoint store. Recheck the combined project after integration.

## Inspect in the terminal

Agent events are tool rows in the conversation. Creation shows as `AGENTS`,
the other events under the collaborator's `AGENT N` name: sent and received
assignments, returns, reviews, stops, closures and unknown outcomes. `agent`
takes the same optional `work` label as the discovery tools, so its events and
inspection calls join a Work chapter with related calls; agent members keep one
row each. An event written by an agent call joins that call's chapter; another
event that happens during a turn joins the chapter it follows. One that happens
outside a turn, such as a return while the main agent is idle or an unknown
outcome found on resume, or after the turn's prose, is a lone row at its
position. Each event records these facts when it happens, so its place does not
depend on when the terminal sees it, and a resumed conversation shows it where
it was shown live. Events never split prose that is still streaming; they land
before it. The transcript shows no clock times. Failed checks and failed runs
are red, unknown outcomes amber and routine waiting quiet. `Ctrl+O` opens the
latest chapter or row and shows every agent event as its full receipt:
instruction, working space, changed files with `+` and `−` line counts, report
and review state. Earlier receipts retain their original snapshots.

`/agents` opens a filterable list of open assignments below the composer. Each
row names the collaborator with its `AGENT N` badge and title, with its state in
the fact column: running time, returned or failed work awaiting the main
agent's review, or an unknown outcome. Enter goes inside the agent: its own
conversation replaces the main column, drawn like the main conversation and read
only. It shows each received assignment as a request, then its calls, chapters
and messages in the order they happened, and the return receipt last. File
edits show their hunks as in the main conversation, read from the agent's own
saved history; if that history is unavailable, the card keeps its `+` and `−`
line counts. One row
of keys replaces the composer, status row and footer: `Esc back to agents`,
`↑↓ scroll` and, while the agent works, `End follow`. The view follows new
output; scrolling pauses it and End resumes following. Esc returns from the
agent to the list, then to the main conversation with its draft and caret. The
list says `Esc close`. While `/agents` is open, Esc does not interrupt the main
turn, and the readback shortcut is Alt+End. There is no child chat or
task-control UI. If the main agent closes the inspected assignment, the view
returns to the remaining open assignments.

While work is running or needs attention, the existing footer shows a quiet
`AGENTS` badge and counts. Failed and uncertain outcomes remain visible. The
context gauge stays; model details yield on narrow terminals. No extra footer row
is added. The normal footer returns once no work or review is pending; completed
assignments leave `/agents` when the main agent closes them. Their creation,
return, review and closure receipts remain at their original points in history.

Resuming a conversation with returned reports in open assignments lets the main
agent continue their review and closure. This also handles older sessions whose
results were returned without a recorded close. Resume does not mark reports as
reviewed itself. Closed assignments stay in history and do not restart the model.

In `--plain`, `/agents` prints the list and `/agents <number>` prints an individual
record by its displayed number. `/agents aN` resolves the unique internal ID;
closing that agent never redirects the ID to a new holder of its old number.
The line-based interface does not poll while waiting for stdin; live
inspection and automatic idle continuation are TUI features. For single prompts,
the model should wait for and review delegated work before ending its turn.

## Stop, close and recover

Leaving a conversation or shutting down cancels its active workers and waits for
their termination. Isolated copies remain until explicitly closed; stopping work
does not discard its files. Child histories live under the parent session's
collaboration storage and do not appear as independently resumable main sessions.

After an interrupted process, a saved running assignment becomes `unknown`;
Jecode does not replay it or restart the worker. Inspect its files and saved
evidence before deciding what to retry. Closed assignments are not restored to
the active list or repeated in the model's current collaborator context. Their
historical receipts and saved child histories remain on disk. An unavailable copy
is reported rather than preventing the
parent conversation from opening.

An isolated copy is prepared just before the receipt that records its
assignment. If Jecode stops in between, no assignment owns the copy. Resuming
that conversation removes such copies and their Git registrations, removing
links inside without following them. Only the resumed conversation's own
collaborator storage is examined, while its session lock keeps other
conversations out, and copies of its saved assignments stay. A copy whose
ownership marker does not name this project and assignment is left in place
and reported in the resume status; new assignments skip its ID.

Deleting a parent conversation retains its collaborator files, child histories
and a recovery record at the location reported by the deletion result. This
preserves isolated work that may not have been integrated. Use `agent close`
before deleting the conversation to discard an isolated working copy explicitly;
retained histories are still identified separately in the deletion result.
