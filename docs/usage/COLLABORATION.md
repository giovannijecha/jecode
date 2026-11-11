# Collaborating agents

The main agent can delegate focused assignments to collaborators: separate
agents that work in parallel and report back. You keep talking to the main
agent; it writes the assignments, follows their progress, reviews what comes
back and decides what to keep. You can watch with `/agents`, but reviewing and
closing assignments is the main agent's job.

The main agent delegates when you ask or when the work splits into independent
parts. Ask if you want collaborators for smaller work.

## How collaborators work

Each collaborator starts with its own conversation and the main agent's current
model, effort and routing. It receives its assignment and any context the main
agent supplies, plus the project's `AGENTS.md` instructions and your personal
skills. It does not receive the main
conversation. Later model changes in the main conversation do not affect
collaborators that already exist.

Collaborators have every tool the main agent has except `agent` and `ask_user`,
including `mcp`, `lsp` and `steps`, and follow the same permission level and
approvals. They cannot delegate further and cannot ask you questions; they
report questions to the main agent. Each collaborator has its own processes,
browsers, temporary files and history. Within one conversation `tmp:` and
`output:` references still resolve between a main agent and its collaborators
(see [Tool reference](TOOL_REFERENCE.md#read)), and `tmp:<agent>:path` lets the
main agent name one collaborator's temporary file. Those reads never write.

Attachments are shared: the main agent can pass an `attachment:ID` in an
assignment, and files a collaborator imports are readable by the main agent.

## Shared or isolated workspace

Every assignment chooses where it works:

- **`shared`** works directly in your project. It suits analysis and coordinated
  changes, but collaborators can affect the same files and external state, so
  the main agent must give them compatible scopes and check the combined result.
- **`isolated`** works in a private copy of the project in Jecode's storage. The
  main agent compares the copy with your project and applies selected files
  only after review.

An isolated copy takes the project's current files, including uncommitted
changes, new files, empty directories and files Git ignores. Git is not needed
and no commit or branch is created. The copy leaves out `.git` and `.jecode`
directories at every level, and `target` and `.tmp` directories at the project
root. Entries the copy cannot hold are listed as `omitted_from_copy` in the
result, with up to 50 named paths:

- sockets, pipes and devices;
- names that are not UTF-8 or contain `:` or `\`;
- files that cannot be read, such as files locked by another process;
- files replaced while they were being copied;
- directories that cannot be listed (the directory itself is still created);
- links that cannot be created, such as symbolic links or junctions on Windows
  without Developer Mode or administrator rights.

Links pointing inside the project are adjusted to point inside the copy.
Commands that depend on omitted links, such as a package manager's linked
`node_modules`, may need them recreated in the copy, for example by reinstalling.

Isolation applies to files only. Commands and browsers in an isolated copy have
the same network and system access as anywhere else.

## Assignment lifecycle

1. **Create.** The main agent creates one or more assignments at a time, each
   with a title, instructions, a workspace and optional context. The interface
   numbers them `AGENT 1`, `AGENT 2` and so on. At most eight collaborators of a
   conversation work at once; the others wait, showing `Waiting for a free
   slot`, and start in the order they were created. Stopping a waiting
   collaborator ends it before it starts.
2. **Update.** While a collaborator works, the main agent can send revised
   instructions or new context. The collaborator receives the new version at its
   next model request; a tool call already running finishes first. Reports say
   which version they answer.
3. **Return.** The collaborator reports what it did and what it checked. If the
   main agent is idle in the TUI, a return wakes it to examine the result,
   unless you have a message queued or explicitly stopped the main turn.
4. **Review.** The main agent inspects the evidence, corrects or integrates the
   work and records its assessment. Recording a review runs no tests and does
   not integrate anything.
5. **Close.** Closing releases the collaborator, removes its isolated copy and
   removes it from `/agents`. A closed collaborator cannot take more work; the
   main agent creates a new assignment instead.

For isolated work, `diff` shows changed files against the starting state and
your current project, and `integrate` applies explicitly selected files. A file
you changed in the meantime is reported as a conflict instead of being
overwritten. Integration works file by file, not as one transaction, and
handles ordinary file additions, edits and deletions. Check the combined
project afterwards.

For shared work, `diff` lists recorded changed files. With `path`, it pages the
collaborator's recorded edit hunks in call order, including multi-file edits.
This is edit history, not a separate baseline or a complete project diff:
command changes and later writes may be absent, and an edit is omitted when a
provider reused its call ID for other files. Read the current files when
reviewing. A path with no recorded edit is refused. Shared work needs no
integration.

### Agent actions

| Action | Fields | Behavior |
| --- | --- | --- |
| `create` | `tasks`: one or more items with `title`, `instructions`, `workspace` and optional `context` | Queues collaborators, starts up to eight turns at once and returns IDs such as `a1`. |
| `update` | `id` and `instructions` and/or `context` | Sends a new version; omitted fields keep their last values. Also starts follow-up work on a returned collaborator. |
| `list` | — | Open assignments. |
| `inspect` | `id` | Activity, assignment versions and report evidence. |
| `wait` | optional `id`, `timeout_ms` (0–60,000) | Waits for running work; stays cancellable. |
| `stop` | `id` | Requests cancellation. |
| `review` | `id`, `report_version`, `note` | Records the main agent's assessment of the latest report. |
| `diff` | `id`; optional `path`, `offset`, `limit` | Isolated: compares the copy with its start and your project, with a review token. Shared: recorded changed files and paged edit history, without a review token. Use the returned path and `next_offset` for subsequent pages. Pages default to 16,384 bytes, at most 65,536. |
| `integrate` | `id`, `paths`, `expected_review_token` | Applies selected files after rechecking for conflicts. |
| `close` | `id` | Releases the collaborator and its isolated copy. |

Collaborator IDs are never reused within a conversation. The displayed number is
the lowest one not held by an open collaborator, so `AGENT 1` can appear again
after the first one is closed.

## Watch in the TUI

Collaboration events appear as rows in the conversation: `AGENTS` when
collaborators are created, then `AGENT N` rows for sent and received
assignments, returns, reviews, stops and closures. Failures show in red and
uncertain outcomes in amber. Ctrl+O shows each event's full receipt: the
instructions, workspace, changed files with line counts, report and review
state. See [Tool activity](../tui/TOOL_ACTIVITY.md#collaborators).

While collaborators work or need review, the footer shows an `AGENTS` badge with
counts.

`/agents` opens a list of open assignments with their state: running time,
returned or failed work awaiting review, or an unknown outcome. Press Enter on
one to read its conversation, drawn like the main one but read-only: its
assignments, tool calls, messages and final report, with file edits shown as
diffs. The view follows new output; scroll up to pause and press Alt+End to
return to the bottom. Esc goes back to the list, then to your draft. While
`/agents` is open, Esc does not stop the main turn. Opening a result does not
mark it reviewed.

In `--plain`, `/agents` prints the list and `/agents NUMBER` prints one record.
Plain chat does not watch collaborators while waiting for your input, so a
returned collaborator does not wake the main agent there. In a single prompt,
the main agent should wait for and review delegated work before it finishes.

## Stop, close and recover

Leaving a conversation or quitting stops its running collaborators and waits
for them to end. Stopping does not discard an isolated copy; only closing does.

If Jecode stops unexpectedly, a collaborator that was running is marked
**unknown** on resume. It is not restarted or replayed; check its files and
evidence before deciding what to retry. When you resume a conversation with
returned but unreviewed work, the main agent continues the review. Closed
assignments stay in history and do not trigger any model request.

An isolated copy prepared just before Jecode stopped, but never recorded as
belonging to an assignment, is removed when the conversation is resumed. A copy
whose ownership cannot be confirmed is left in place and reported.

Deleting a parent conversation keeps its collaborator files, histories and a
recovery record, and the deletion result says where, so unintegrated isolated
work is not lost. To discard an isolated copy explicitly, have the main agent
close the assignment before you delete the conversation.

Collaborator data lives with the conversation, under
`~/.jecode/sessions/<project>/<session>/collaboration/`. Each collaborator's own
history, command output and temporary files are kept in its session folder
there, in the same layout as a main conversation. Collaborator histories cannot
be opened from `/resume`.
