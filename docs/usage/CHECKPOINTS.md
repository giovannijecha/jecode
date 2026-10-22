# Checkpoints

Jecode keeps local file history so you or the model can inspect and undo selected
changes. It is separate from Git and from [saved conversations](SESSIONS.md).
The archive is private to your Jecode home, but it can contain unredacted
project files, including Git-ignored secrets such as `.env`.

## When files are protected

Each canonical project directory has one archive under
`~/.jecode/checkpoints/<project-name>-<id>/`, shared by its Jecode sessions.
Opening a project or starting a turn does not scan its files. Protection is
acquired when a tool needs it:

| Action | Preimage saved before the change |
| --- | --- |
| `write` or `edit` | The exact file path, including an absent path before creation. |
| Bash with `protect` | The declared files and complete recursive directory scopes. |
| `checkpoint capture` | Explicit files, directories or the whole workspace when requested. |
| Restore | The current states of paths it will change, for undo. |

For Bash, pass workspace-relative paths in `protect`, for example
`{"command":"mv src/old.rs src/new.rs","protect":["src/old.rs","src/new.rs"]}`.
An existing directory is recursive. Use `"generated/"` to cover a directory
that does not yet exist or will replace a file. Use `"."` only when you want
the entire eligible workspace captured before launch. Bash has normal system
access: its working directory does not confine absolute paths, and `protect`
does not intercept writes outside the declared scopes.

The preimage is made durable before Bash launches. A failed capture does not
silently become a save: the command can still run and reports `partial` or
`none` with the affected paths and reasons. Cancelling before launch stops the
command. Bash's `changed` paths are observations within the declared scope,
including changes seen after a persistent process exits. Their writer is
unknown; another process may have edited them. Nothing outside the selected
scope is treated as known absent.

`write` and `edit` results use `checkpoint.status: saved`, `not_tracked` for
excluded or temporary paths, or `unprotected` with a reason. Bash uses
`saved`, `partial` or `none`. A saved Bash or capture result includes a `before`
snapshot and a `group`; Bash also returns an `after` observation reference.
The `before` snapshot restores that step's preimages. The group restores the
first saved state of each path in the current attempt.
Use `after` for comparisons. Collection can release an unpinned observation;
use the saved `before` snapshot or recovery group for recovery.

## Restore and accept

One active recovery group belongs to each conversation. Its successive
snapshots cover the paths saved by that conversation's changes. A successful
Bash exit does not accept or discard its group. The model explicitly calls
`checkpoint` with `{"action":"accept"}` when the attempt is complete; it can
name a group or snapshot with `ref`. The group then keeps only its first saved
state per path, for a fixed period of at most 24 hours from acceptance.
Intermediate snapshots stop being available unless you pinned one. Active
groups expire after seven days of meaningful inactivity by default; a running
persistent process renews its active group. Only the user controls pins.

Inspect before restoring. A restore compares the live state with an expected
state and rechecks it immediately before replacing or deleting each path. A
model restore defaults to the conversation's last known version after the
target; it does not assume Bash wrote an observed change. When undoing a Bash
step after inspecting its result, pass its returned `after` as the explicit
`expected` reference. `expected: "latest"` accepts the state observed when
planning. A conflict leaves the affected path untouched and reports the
current version for review. Earlier paths may already have changed; inspect
`restored`, `not_restored` and `undo` in a partial result. The returned `undo`
is based on the states saved before installation.

Restore resolves verified filesystem aliases to the saved path spelling. If a
path's spelling is ambiguous, use the exact name shown in the archive;
an unverified alias is not evidence that the saved file was absent.

Restoring a saved directory can remove children created within that complete
scope after its `before` snapshot. It never interprets a path outside complete
captured scope as absent. Excluded, unreadable and oversized paths are not
protected by that capture. An exact file capture does not cover future siblings
or children; declare a directory scope for those.

## Scope, policy and capacity

Git ignore rules do not determine checkpoint scope. The defaults exclude VCS
metadata and generated directories such as `.git`, `target`, `node_modules`,
`.venv`, `.cache` and `.jecode-output`; `.env` is eligible. Edit this archive's
`policy.json` to change the rules and limits:

```json
{
  "version": 3,
  "max_file_bytes": null,
  "budget_bytes": 1073741824,
  "min_free_bytes": 268435456,
  "recovery_accepted_hours": 24,
  "recovery_inactive_days": 7,
  "exclude": [".git", "target", "node_modules", "*.log"],
  "include": ["keep.log"]
}
```

`max_file_bytes: null` is the default: there is no configured per-file
ceiling. A numeric value imposes one; `config --max-file unlimited` restores
the default. The 1 GiB `budget_bytes` is a soft retention target, not a hard
archive cap. `min_free_bytes` asks Jecode to leave a free-space margin before
archive writes; disk space can still change between a check and a write.
`recovery_accepted_hours` cannot exceed 24. Set `recovery_inactive_days` to
`null` to disable inactivity expiry, or to a positive number to change it.

A rule without `/` matches a component name; one with `/` matches the relative
path. `*` and `?` stay in one component, `**` spans components, and a trailing
`/` limits an exclusion to directories. An `include` rule can override an
exclusion, but an excluded parent must itself be included to visit children.
Omitting `exclude` uses the full built-in defaults. Existing numeric limits
from legacy policies are preserved; Jecode does not guess whether they were
user choices. Only an archive selected for use is migrated to the current
layout, in place and after older readers release it.

Large files use bounded readers and deduplicated blocks. Metadata, available
memory, permissions, filesystem support and free disk space still matter.
Check each `checkpoint` result: incomplete capture is reported as partial or
unprotected. Bash and explicit capture show the first 20 partial diagnostics;
`partial_count` and `partial_omitted` report larger totals without filling the
conversation with every affected path. `verify` deliberately rereads the
eligible workspace; normal
startup and turns do not require that scan. A snapshot is an ordered set of
observations, not an atomic multi-file filesystem image.

## Commands

`jecode checkpoints` works from the project directory without a model key,
provider or network. `/checkpoints` accepts the same words in a conversation.

```text
status                              archive state, limits and activity
list [PATH...] [--limit N]          points or path versions
capture PATH...                     save files or recursive directories
accept [cp-N]                       accept a recovery group
inspect cp-N [PATH...]              target state and restore preview
show cp-N PATH                      recorded content
milestone LABEL [--pin]             named user point
restore cp-N PATH... [--exact-paths] [--preview] [--expected cp-M|latest]
restore-all cp-N [--preview] [--expected cp-M|latest]
pin cp-N [LABEL] | unpin cp-N       user-controlled retention
config [--budget SIZE] [--max-file SIZE|unlimited]
gc                                  compact eligible history
verify                              reread eligible workspace files
```

Examples: `jecode checkpoints capture src/ generated/` saves a recursive
source directory and a new directory scope; `capture .` deliberately includes
the eligible workspace. `restore cp-42 src/ --preview` shows what would change.
`--exact-paths` selects only named entries. `restore-all` uses the target's
recorded scope. The model's `checkpoint` tool supports `status`, `capture`,
`accept`, `list`, `inspect`, `milestone`, `restore` and `restore_all`; user-only
commands control pins, policy, collection and verification.

A milestone records the observed states of already tracked paths. Capture
additional files or directories first when you want the point to cover them.

`read` accepts `checkpoint:cp-N:path` for content retained by a recoverable
reference. A version listed for comparison is not necessarily a retained
preimage. Notices about another session's restore, policy change or recovered
operation reach the next model request; a changed `JECODE.md` is reloaded.

## Retention and limits

Collection retains active recovery states, accepted groups until their fixed
deadline, user pins and unfinished operations. It can release intermediate
versions and objects while keeping lightweight comparison metadata. `gc`
compacts eligible history; old generations remain readable until their active
readers release them. Compaction can temporarily need both generations on
disk, so neither the budget nor the free-space margin reserves disk capacity.

Inactive archive cleanup runs in background maintenance; opening a current
archive does not synchronously sweep or migrate other projects. Cleanup does
not delete workspace files or conversation saves. Identity, policy and the
sequence watermark survive cleanup, so an old `cp-N` cannot name unrelated
new data. Legacy archives migrate only when selected; migration waits for old
readers and can resume after interruption.

Recovery deadlines are checked on access. Physical removal runs while Jecode
is active, including an hourly sweep of closed current-format archives; no
operating-system service runs while Jecode is closed. Readers, user pins and
unfinished recovery can keep objects on disk after an ordinary deadline.

Retention uses monotonic elapsed time while a process runs, so moving the
system clock does not immediately expire open recovery groups. Across process
restarts, persisted deadlines still depend on the system clock being correct.

On Windows, free-space checks use an owned helper and `GetDiskFreeSpaceExW`
on the actual archive directory, including mounted volumes and UNC paths.
The helper uses the same system C# compiler/cache boundary as process control.
Unix checks use the system `df -Pk` tool. A failed query is reported rather
than treated as an unlimited amount of free space. Estimates are refreshed
in the background and account for this process's journal writes; other writers
can still consume space before the OS accepts a write. Small terminal recovery
records can use the free-space margin so a failed capture can be closed.

Checkpoints are local history, not a backup. Writes outside declared Bash
scopes or between two observations are not captured. Symbolic links are saved
as links; directory traversal does not follow them outside the workspace.
For a command that writes through a symbolic link, declare its target path too;
protecting only the link saves its directory entry, not the target's contents.
Restores preserve file bytes, permissions and link targets, but not hard-link
relationships, extended attributes, ACLs or timestamps. Another writer can
still race the final restore recheck. On Unix, completed archive writes sync
their affected directory entries; the Rust standard library does not expose
the same directory flush on Windows, so completion there is not a power-loss
guarantee.
