# Checkpoints

Checkpoints let the model recover files after a modifying tool call goes wrong.
Each reference identifies the state saved **before one call**. They are local,
independent of Git and conversations, and expire 24 hours after capture.

## What is saved

`write` and `edit` automatically save their file's previous bytes and mode. A new
file records that the path was absent. An edit that finds no unique match does
not create a checkpoint. Successful and failed calls have the same expiry;
Jecode cannot infer whether a successful command produced the intended result.

`agent integrate` captures the selected main-project paths before applying an
isolated collaborator's changes. Its result includes the same checkpoint receipt
and capture diagnostics. Collaborators have their own checkpoint stores for
their file and command tools; see [collaboration](COLLABORATION.md).

Bash can modify several files, so the model declares its protection scope:

```json
{"command":"./update.sh","protect":["src","Cargo.toml"]}
```

Files are exact targets; directories include their eligible descendants. Use
`"."` for the eligible workspace when the command's scope is broad. Without
`protect`, Bash creates no checkpoint. Jecode does not parse shell commands to
predict writes, and protection does not constrain what Bash may change.
Persistent processes use their launch snapshot, with the same 24-hour deadline.
There is no post-command scan, automatic rollback or success-based deletion.

A receipt contains `ref`, `saved_paths`, `expires_at` (Unix milliseconds) and
`status`: `saved`, `partial` or `none`. Partial receipts include bounded reasons
and the total issue count. A saved receipt proves a preimage was durable before
execution; it does not prove that execution started, finished or succeeded.
Capture failures are reported without preventing an otherwise valid tool call.
Cancellation before launch prevents the call from starting.

Startup, new turns, reads and commands without protection do not scan or archive
the workspace. There is no generic version history, acceptance action, recovery
group, milestone, pin, watcher or configurable retention policy.

## Inspect and recover

The `checkpoint` tool has three actions:

| Action | Result |
| --- | --- |
| `list` | Recent unexpired calls; default 20, `limit` up to 200. |
| `inspect` | Saved/current states, proposed changes and a current-state `expected` token. |
| `restore` | Restores selected paths, or the saved scope of the call when `paths` is omitted. |

For example, after a bad edit:

```json
{"action":"inspect","ref":"rc-ID","paths":["src/main.rs"]}
{"action":"restore","ref":"rc-ID","paths":["src/main.rs"],"expected":"TOKEN_FROM_INSPECT"}
```

A restore requires an `expected` token from inspection of the same selected
paths. The explicit value `"latest"` instead acknowledges their current state.
If the inspected state changed, restore reports a conflict. `preview: true`
reports the plan without changing files or creating an undo.

Before restoring, Jecode verifies saved content and durably saves the current
states as an undo. It rechecks the digest of current bytes before each
installation. The returned undo includes only paths actually changed, including
a failed installation if it modified that path. Use its supplied `ref`, `paths` and
`expected` to undo. Restore is per path, not an atomic transaction across files;
a partial result reports restored, conflicted, failed and skipped paths.

The original preimage and an undo remain independently available until their
own 24-hour deadlines. Restoring never renews the original reference. A crash
before a tool starts may leave an unused snapshot; a crash during a mutation
leaves its before snapshot. There is no inferred completion replay.

If a restore process dies before returning its result, find its undo snapshot
in `list` and inspect it before choosing paths. No per-path crash progress log
is kept; the pack contains the states saved for the planned restore.

Read a saved file through the normal `read` tool:

```json
{"path":"checkpoint:rc-ID:src/main.rs"}
```

Paging works as for workspace text. The requested file's checksum is verified;
reading it does not load other saved payloads. Exact recorded names remain
usable after deletion, including non-ASCII names. Supported live aliases resolve
to the saved entry without traversing a final link.

## Scope and storage

Snapshots are private files under
`~/.jecode/checkpoints/<project-name>-<canonical-root-id>/`. Each call creates
one immutable `rc-ID.rcp` file directly in that workspace directory, containing
its streamed preimages and checked metadata. New packs use format 4; formats 2
and 3 remain readable until their original expiry. Jecode keeps `workspace.id`,
`identity.lock` and path leases inside `.internal/` in the same workspace
directory. There are no daily or conversation subdirectories. Configured TUI
and CLI launches create the workspace checkpoint directory; provider-free
`jecode checkpoints` actions also create it when needed.
There is no global journal replay, content-object graph, delta chain or copying
compaction. Repeated calls store independent copies; frequently editing a very
large file can consume more space than deduplicated history.

File payloads use fixed-size I/O buffers. Metadata and traversal memory grow
with the number of selected paths. There is no configured file-size or path-count
ceiling; memory, disk capacity and permissions still constrain capture. Admission
keeps a 256 MiB free-space margin based on advisory OS samples. A failed or
unreadable path is reported as partial coverage and never as protected absence.

Recursive discovery skips version-control metadata, generated/cache directories
such as `target`, `node_modules`, `.venv` and `.cache`, directories Git ignores,
and internal recovery storage. Ignore rules come from `.gitignore` files and the
repository's `info/exclude`, read directly so Git need not be installed; the
global excludes file is not read. Only directories are skipped: an ignored file
such as `.env` is still saved. The captured rules and skipped directories are
recorded, so later rule changes cannot widen or narrow the saved scope. This
also covers ignored directories created after capture. An explicitly protected file
or directory can select generated or ignored content; internal storage stays
excluded. Symlinks are saved as links without traversing their targets.
Unsupported entries, unreadable directories and untraversed Windows reparse
directories make their scope incomplete. A complete directory proof is recorded
only after capture and a membership recheck agree. Only such a proof can
establish that a later-created eligible child should be deleted by restore. A
later selection cannot expand the scope originally captured.
An explicitly protected path that is absent throughout capture also proves its
eligible subtree was absent, so a newly created directory can be recovered too.

Path leases coordinate Jecode sessions on the same archive. Arbitrary external
programs do not honor them; restores check live states and bytes, but filesystem
operations cannot provide a transaction against an uncooperative external writer.
Packs and staged files are synced before they are renamed into place. Like
`write` and `edit`, checkpoints do not sync directory entries, so a power loss
right after a call can lose the newest pack or installation.

## Expiry

Every snapshot, including an undo, expires 24 hours after its creation timestamp
and refuses new access. Reads, commands, processes, activity and restarts do not
extend it. Time is sampled from the wall clock so suspension counts; persisted
timestamps do not decrease within a process. Correct UTC time is required.

While Jecode runs, maintenance checks recognized workspace checkpoint directories
once a minute and deletes expired packs individually. Readers temporarily hold
a shared lease, so physical removal may wait until a reader releases it. Offline
commands also sweep expired packs. When Jecode is closed, cleanup runs on the
next launch; no background system service is installed. The 24-hour deadline
applies to packs, not workspace identity or lease metadata. Workspace files and
unrecognized data are never removed by expiry.

## Local commands and previous archives

`/checkpoints` and the provider-free CLI use the same actions:

```text
jecode checkpoints list
jecode checkpoints inspect rc-ID src/main.rs
jecode checkpoints show rc-ID src/main.rs
jecode checkpoints show rc-ID src/main.rs --offset 201 --limit 200
jecode checkpoints restore rc-ID src/main.rs --preview --expected latest
jecode checkpoints restore rc-ID src/main.rs --expected TOKEN_FROM_INSPECT
```

Omit paths to inspect or restore the call's saved scope. `show` streams saved
text; binary files can be restored. Commands need no model, provider or network.
In the conversation, `show` displays a bounded page with a next-page command.
Explicit paging flags work in both interfaces. Suggested offline commands use
PowerShell quoting on Windows and POSIX shell quoting on Unix; `--` separates
paths beginning with dashes from options.
