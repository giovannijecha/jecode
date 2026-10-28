# Collaborating agents

The main agent can use `agent` to delegate focused assignments, revise them while
work is in progress, inspect evidence and review the result. You continue talking
to the main agent. `/agents` provides read-only inspection without making a model
request or marking a result reviewed.

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
`a1`; the interface displays `AGENT 1`. Each control call is serialized with other
main-agent tools, even with `parallel:true`. The workers continue independently.

| Action | Required fields and behavior |
| --- | --- |
| `create` | `tasks`, each with `title`, `instructions`, `workspace`; optional focused `context`. |
| `update` | `id`, `instructions`; optional `context`. Sends a new version during work or starts follow-up work on an available returned worker. |
| `list` | Returns current session records. |
| `inspect` | `id`. Returns activity, assignment versions and report evidence; does not review it. |
| `wait` | Optional `id` and `timeout_ms` (0–60000). Waits for selected running work, remaining cancellable. |
| `stop` | `id`. Requests cancellation; a request is distinct from a confirmed stop. |
| `review` | `id`, `report_version`, `note`. Records the main agent's assessment of the latest returned assignment. |
| `diff` | `id`. Compares an isolated workspace against its starting state and the current main project. Optional `path`, byte `offset` and `limit` page before/after content. |
| `integrate` | `id`, explicit `paths`, `expected_review_token` from `diff`. Applies selected changes with conflict checks. |
| `close` | `id`. Stops the worker and removes its owned isolated copy, retaining the historical session record. Close after preserving needed work. |

Supply the complete revised assignment in `update`. An update being sent does
not mean it has been received. Running workers consume
updates at model boundaries; a tool already executing may finish first. Reports
identify their received assignment version. Older reports remain evidence of
older work and cannot review a newer active assignment.

Returned work still needs the main agent's inspection, correction and verification.
`review` records that assessment; it does not run tests or imply integration.
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

Creation appears as an `AGENTS` group, expandable with the existing `Ctrl+O`
interaction. Sent updates, received instructions and returned work produce new
chronological receipts. Earlier receipts retain their original snapshots.

`/agents` opens a filterable list below the composer. Enter opens a record;
Activity, Assignment and Result tabs show visible progress, tool activity,
instructions, working space and reported evidence. Activity follows new output;
scrolling pauses it and End resumes following. Esc returns from detail to list,
then restores the main draft and caret. There is no child chat or task-control UI.

While work is running or needs attention, the existing footer shows a quiet
`AGENTS` badge and counts. Failed and uncertain outcomes remain visible. The
context gauge stays; model details yield on narrow terminals. No extra footer row
is added. The normal footer returns once no work or review is pending; completed
records remain accessible through `/agents`.

In `--plain`, `/agents` prints the list and `/agents <number>` prints an individual
record. The line-based interface does not poll while waiting for stdin; live
inspection and automatic idle continuation are TUI features. For single prompts,
the model should wait for and review delegated work before ending its turn.

## Stop, close and recover

Leaving a conversation or shutting down cancels its active workers and waits for
their termination. Isolated copies remain until explicitly closed; stopping work
does not discard its files. Child histories live under the parent session's
collaboration storage and do not appear as independently resumable main sessions.

After an interrupted process, a saved running assignment becomes `unknown`;
Jecode does not replay it or restart the worker. Inspect its files and saved
evidence before deciding what to retry. Closed or unavailable workspaces retain
historical records; an unavailable copy is reported rather than preventing the
parent conversation from opening. An interrupted workspace setup can be retried
only after verifying ownership of the incomplete copy. Complete copies and
foreign directories are preserved.

Deleting a parent conversation retains its collaborator files, child histories
and a recovery record at the location reported by the deletion result. This
preserves isolated work that may not have been integrated. Use `agent close`
before deleting the conversation to discard an isolated working copy explicitly;
retained histories are still identified separately in the deletion result.
