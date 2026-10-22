# Checkpoints

Jecode keeps a private local history of your project's files so you or the
model can inspect and undo changes. It records changes made by Jecode's tools,
by commands they start, by editors and by other Jecode sessions, including
Git-ignored files such as `.env`. Checkpoints are separate from Git and never
read or change the Git index, ignore files or repository.

These workspace checkpoints are not the conversation saves described in
[saved sessions](SESSIONS.md); they restore files, not messages.

## What is saved and when

Each canonical project directory has one archive under
`~/.jecode/checkpoints/<project-name>-<id>/`, shared by every Jecode process
that opens it. The first session captures the scope in the background. Bash
waits for this capture before recording its starting point; cancellation also
interrupts that wait. Later commands reconcile changes instead of recopying
unchanged content. A failed save still allows the tool to run with an explicit
`unprotected` result. Restores cover a path once its state has been captured.

| Moment | What Jecode records |
| --- | --- |
| Before `write`, `edit` or a restore changes a file | Its previous state, durably, before the file changes. A restore stages file replacements before installation. |
| Start of each turn and before each Bash command | A durable point (`cp-N`) in the observed history. An unchanged history reuses its last point. |
| During and after commands | Changed files observed while the command ran; background processes keep being observed. |
| A `milestone` | A named point, optionally pinned. |
| An explicit `capture` | The current states of named files or links when an exact preimage is needed. |

`write`, `edit` and `bash` results carry a `checkpoint` object:

For Bash, `changed` lists paths observed during the command. It does not
identify which process wrote them; editors and other sessions may run at the
same time. Recorded versions found through filesystem observation have origin
`external` because their writer is unknown.

| `status` | Meaning |
| --- | --- |
| `saved` | The previous state is at `before`; `after` is the recorded result. |
| `not_tracked` | The path is outside the workspace or excluded by the scope; `reason` says which. |
| `unprotected` | The previous state could not be saved; `reason` explains why. The tool call still ran. |

Successful `write` and `edit` calls to `tmp:` session storage report
`not_tracked`: that storage is outside the project checkpoint scope.

A failed save does not block `write`, `edit` or Bash. Saves are quiet in the TUI: a
tool card shows a warning only when a checkpoint problem needs attention, and
`Ctrl+O` shows a call's `before → after` points.

### How changes are noticed

On Windows, an owned .NET helper reports file changes as they happen. Jecode
compiles it once into its helper cache with the system C# compiler, as it does
for the process supervisor. Ordinary points drain changes already reported
without a fixed delay. Bash preparation and completion briefly allow notifications to settle;
that wait and the helper's input echo are not filesystem barriers. If the
helper is unavailable, Jecode scans instead. Its pending path queue is bounded;
overflow requests reconciliation rather than growing without limit.

On Linux and macOS, Jecode scans at each point, and every few seconds while a
background process runs. A scan compares size, modification time, change time
and file identity, then rereads changed files. In selective mode it examines
recorded paths, without walking the large tree at every point.

After `write` or `edit` succeeds, Jecode records the content the tool wrote
without reading the file back. The file is reread in the background, or at
the latest before the next point, so another program's change made at the same
moment is still recorded.

When a session opens an archive, it reconciles changes made while no session
was running. Interactive sessions on Windows also reread each protected file once, to
find edits that preserved size and modification time.

## Scope and capacity

The scope is Jecode's own: every file under the project directory except the
default exclusions, regardless of Git ignore rules. `.env` is included by
default; add a policy exclusion if the local archive should omit it.

| Setting | Default |
| --- | --- |
| Excluded names | `.git`, `.hg`, `.svn`, `.jj`, `.bzr`, `_darcs`, `.jecode-output`, `.jecode-*.tmp`, `target`, `node_modules`, `__pycache__`, `.venv`, `.mypy_cache`, `.pytest_cache`, `.ruff_cache`, `.tox`, `.gradle`, `.next`, `.nuxt`, `.turbo`, `.parcel-cache`, `.svelte-kit`, `.terraform`, `.cache` |
| Maximum captured file | No configured limit; an explicit limit remains available |
| Retention budget | 1 GiB soft target for releasing old history; required current states, pins and recovery can exceed it |
| Discovery | No configured path count or content-size ceiling; generated subtrees are pruned |
| Inactive history | Released automatically after seven inactive days, subject to active-reader, pin and recovery protection |

Symbolic links are recorded as links and directories by their permissions.
Unreadable files retain the reason; their content cannot be restored. Available
disk space, filesystem support, permissions and memory for path/version/object metadata
still constrain the archive. Check `status` and each tool's protection result.

Edit the archive's `policy.json` to change the scope:

```json
{
  "version": 2,
  "max_file_bytes": null,
  "budget_bytes": 1073741824,
  "exclude": [".git", "target", "node_modules", "*.log", "data/raw/"],
  "include": ["keep.log"]
}
```

A rule without `/` matches any path component name, such as `target` or
`*.log`. A rule with `/` matches the whole project-relative path, such as
`docs/build`. A trailing `/` limits a rule to directories. `*` and `?` stay
within one component; `**` spans components. An `include` rule admits a path
that an exclusion would otherwise remove; an excluded directory is not visited
unless an `include` rule matches the directory itself. An omitted `exclude`
uses the defaults above.

Edits apply when a session next opens the archive or records a point. Jecode
rescans and records the change in history; running conversations receive a
notice. Scope changes preserve existing history. `config --budget SIZE` changes
the soft retention target. `config --max-file SIZE` sets an explicit file limit;
`config --max-file unlimited` restores the unlimited default. Policy version 1's
former generated 10 MiB default migrates to unlimited. Other legacy values and
explicit version 2 limits remain configured.

### Large workspaces and files

Discovery visits the eligible tree without the former 50,000-path or estimated
content-size cutoff. Captured content moves through bounded batches. Files
larger than 1 MiB use deduplicated 1 MiB blocks and hierarchical manifests;
acquisition, exact comparisons, historical reads and restore stream the bytes.
Large text edits validate UTF-8 and locate an exact unique match across block
boundaries before staging their replacement. Memory for file contents does not
grow with the complete file size; path, version and block/object metadata remains
in memory.

On Windows, subsequent ordinary points use delivered change notifications.
Linux and macOS scan metadata at each point, then reread changed files. Tree
size therefore still affects their point latency. The first complete acquisition
must read the eligible content on every platform.

`capture` remains useful for a precise preimage or partial coverage caused by
unreadable paths. Name individual workspace-relative files or links; directories
are skipped rather than captured partially. Each `saved` entry has a durable
`saved_at`; `ref` names a point after the saved states. An absent path is saved
as absent, so restoring it after Bash creates it removes it. Explicitly excluded,
unreadable or configured oversized paths have no recoverable preimage. If a
file changes while being captured, inspect the unprotected result and retry.

Points record observed history rather than an atomic filesystem snapshot.
External writers can race after observation. `status`, Bash results and request
context expose incomplete coverage. Unknown prior states are never interpreted
as permission to delete a file. `verify` retries complete discovery and rereads
the scope; a later healthy scan can recover from an interrupted acquisition.

## Commands

Use `jecode checkpoints` from the project directory. It works offline,
without an OpenRouter key, provider, Bash or curl:

```text
status                              archive state, limits and activity
list [PATH...] [--limit N]          points, or the recorded versions of paths
capture PATH...                     save explicit file paths before Bash
inspect cp-N [PATH...] [--exact-paths]  a point and what restoring it would change
show cp-N PATH                      a path's recorded content
milestone LABEL [--pin]             record a named point
restore cp-N PATH... [--exact-paths] [--preview] [--expected cp-M|latest]
restore-all cp-N [--preview] [--expected cp-M|latest]
pin cp-N [LABEL] | unpin cp-N       keep a point regardless of the budget
config [--budget SIZE] [--max-file SIZE]
gc                                  release history beyond the budget now
verify                              reread every file for disguised edits
```

```powershell
jecode checkpoints
jecode checkpoints list src/main.rs
jecode checkpoints capture src/main.rs notes/new.txt
jecode checkpoints inspect cp-42 src/main.rs
jecode checkpoints show cp-42 src/main.rs
jecode checkpoints restore cp-42 src/main.rs --preview
jecode checkpoints restore cp-42 src/main.rs
jecode checkpoints restore-all cp-42
jecode checkpoints milestone "before the refactor" --pin
jecode checkpoints config --budget 512MiB --max-file 5MiB
```

Inside a session, `/checkpoints` takes the same words, such as
`/checkpoints restore cp-42 "notes/a b.txt"`. Double quotes keep spaces.
The TUI shows the result in the information panel without adding a model
message. Archive commands run while the terminal remains responsive; later
conversation commands and prompts wait until the command finishes. They cannot be cancelled midway,
and exiting waits for the current command to finish. Your restores use the
current state as the initial expected version unless you pass `--expected`;
they still check each path again immediately before changing it.

## The model's checkpoint tool

The model uses the `checkpoint` tool with these arguments:

| Argument | Used by | Meaning |
| --- | --- | --- |
| `action` | all | `status`, `capture`, `list`, `inspect`, `milestone`, `restore` or `restore_all`. |
| `ref` | `inspect`, `restore`, `restore_all` | A point such as `cp-42`. |
| `paths` | `capture`, `list`, `inspect`, `restore` | Project-relative paths; `capture` requires 1–200 explicit file or link names. |
| `exact` | `inspect`, `restore` | With explicit `paths`, `true` selects only those entries, without recorded descendants; defaults to `false`. |
| `label`, `pin` | `milestone` | The point's name; `pin: true` keeps it regardless of the budget. |
| `expected` | restores | A point that accepts the current state, or `latest`. |
| `preview` | restores | `true` plans without changing files. |
| `limit` | `list` | 1–200 entries; the default is 20. |

An empty `expected` uses the same conflict checking as an omitted value.
An empty `paths` array means no path filter for queries and `restore_all`;
`capture` and `restore` still require explicit, nonempty paths.
`exact` and `--exact-paths` change path selection only; conflict checking is unchanged.

The `read` tool accepts `checkpoint:cp-N:path` to read a recorded version.
`pin`, `unpin`, `config`, `gc` and `verify` are only for you.

Before the next model request, the model receives a notice when someone else
changed history: your restores, another conversation's restores, scope or
limit changes and recovered interrupted operations. When a change touches a
`JECODE.md`, Jecode also reloads the project instructions.

## Restores and undo

A restore first saves the current state of every affected path and stages file
replacements. Immediately before replacing or deleting a path, it reobserves
that path. If its state changed since planning, the path stays untouched and
the restore stops with a conflict. Earlier paths may already have changed;
inspect `restored`, `not_restored` and `undo` in the result. By default, `restore`
also covers recorded paths below a restored directory; `exact: true` (or
`--exact-paths`) selects only the named entries.
`restore_all` returns the recorded scope and deletes paths known to have been
created after the point. Paths whose prior state was never captured are skipped.

The returned `undo` restores the state before the operation. After partial
progress, it selects only the paths confirmed restored, with `exact: true`, so
untouched descendants of a selected directory stay untouched. A failed step is
recorded from the live state without assuming who wrote it; inspect that path
and `before` before deciding whether to restore it. If no path was confirmed
restored, there is no undo action. A successful full restore uses a compact
`restore_all` undo.

By default, a model restore compares the current version with the conversation's
last version after the target point, or the target if there is none. A mismatch
is reported in `conflicts` with target, expected and current points; it does not
establish who changed the file. Its `retry` value is an `expected` point that
accepts the observed state once it has been inspected; `expected: "latest"`
accepts the current state when planning. Neither bypasses the immediate recheck.
`preview` returns a plan without changing files.

## Storage and retention

Content is deduplicated; small files can use prefix/suffix deltas and large
files reuse unchanged blocks. Collection starts at 40% of the soft retention
budget and targets 25%. The current state, latest named point, pins and
unfinished operations remain protected even when they exceed that target.
`gc` compacts now. Old generations remain readable until their readers release
them. Compaction can temporarily require both old and new generations on disk;
the budget is neither a hard archive cap nor a reservation of free space.

The archive layout separates durable history from live coordination:

```text
<project-name>-<id>/
  workspace.json
  policy.json
  CURRENT
  history/
    CURRENT
    gen-000001/
    gen-000001.lock
  runtime/
    activity/
    owners/
    leases/
    lifecycle.lock
    last-use
    high-water
  store.lock
  gc.lock
```

Lock files are internal process coordination and normally occupy no content
bytes. Existing hashed archive directories keep their name and migrate their
contents in place when no legacy reader is active. Close older sessions before
opening their archive with the new version; new writers wait for migration
rather than appending a new format to legacy history. The migration is resumable.
Root coordination locks stay stable across migration. Every new-layout archive
keeps a root `CURRENT` compatibility marker that rejects older binaries; the
active generation is in `history/CURRENT`. New readable archives also reserve
their old hash-only lookup path with a small identity file, preventing an older
binary from creating a separate archive. Keep these compatibility files intact.
If parallel legacy history already exists, Jecode preserves both copies and
reports the ambiguity.

Jecode checks inactive archives when it opens and periodically while running.
After seven inactive days, it removes unpinned history with no unfinished
recovery or active reader. This never deletes workspace files or conversation
saves. Activity writes and sweeps are coalesced hourly, with a conservative
margin so history cannot expire early; cleanup can occur later than the exact
seven-day boundary. No operating-system scheduler or service is installed.
Opening an expired archive checks it before renewing activity. Archives without
an activity timestamp receive a fresh seven-day grace period. Background
maintenance and automatic collection do not renew user activity. Identity,
scope policy and the sequence watermark
survive cleanup, so old `cp-N` references cannot resolve to unrelated new data.

Filesystem allocation and metadata overhead differ from file lengths. `status`
shows logical archive bytes and the active generation separately. A full disk
or I/O error produces incomplete or unprotected results while normal tools
continue; it never authorizes discarding required current states or pins.

Archives contain unredacted file contents, including secrets in `.env` files,
in private files under your Jecode home. Model-visible results use the same
credential masking as other tools. Deleting `~/.jecode/checkpoints/<id>/`
deletes that project's history.

On Unix, completed archive changes sync their contents and permissions and the
affected directory entries. The Rust standard library does not provide the
same directory flush on Windows, so completion there is not a power-loss
guarantee. Process-kill recovery tests do not simulate a power cut.
Unix restores batch shared parent-directory flushes. If a flush fails, the
result reports the durability failure and installed files under that parent
keep their prepared journal records. After that process exits, recovery checks
their actual state; the records also retain their preimages until then.

## Several sessions

Several Jecode processes can use one archive. File locks order archive
writes and serialize changes to the same path. `status` lists the other live
sessions and what they are doing. Each conversation's restores use its own
changes to detect conflicts.

## Limits

- Checkpoints are local history, not a backup: the archive lives on the same
  machine and disk.
- Changes by other programs are recorded after they happen. If a program
  writes a file twice between two observations, the intermediate version is
  not recorded.
- On Linux and macOS, an edit that keeps a file's size, modification time,
  change time and identity is found only by `verify`. Change time moves with
  every write, so ordinary edits are always found.
- A point names observed history, not an atomic multi-file filesystem image.
  On Windows, a change can miss a point when its notification arrives later;
  it is then recorded in a later point.
  Changes made while the helper restarts are found by a rescan.
- Filesystem notifications and scans identify changed paths, not writers.
  Their recorded versions have origin `external`, including changes observed
  while a Bash command or background process runs.
- The recheck before restore installation narrows the race with other editors,
  but an uncooperative editor can still change a path between the recheck and
  the filesystem operation.
- Restores preserve file bytes, permissions and symbolic link targets. They do
  not reproduce hard-link relationships, extended attributes, ACLs or timestamps.
- An unchanged archive reopens without rereading files, except for the
  Windows interactive reread described above, which takes about 1 s for
  10,000 files.
