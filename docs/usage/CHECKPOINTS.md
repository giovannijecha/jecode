# Checkpoints

A checkpoint is the saved state of files just before one modifying tool call.
If a call goes wrong, you or the model can inspect what changed and put the
files back. Checkpoints are local, independent of Git and of conversations, and
each one expires 24 hours after it was taken.

## What is saved

| Call | Saved before it runs |
| --- | --- |
| `write`, `edit` inside the project | Attempts to save the file's previous bytes and mode, or record that it did not exist. |
| `bash`, `exec` | Only the paths listed in the call's `protect` argument. Without `protect`, nothing. |
| `process write`, `resize`, `interrupt` | Only the paths listed in `protect`. |
| `agent integrate` | The main-project paths a collaborator's changes are about to replace. |

Writes and edits to `tmp:` or approved paths outside the project are not
checkpointed.

For commands, Jecode does not guess what will change. The model declares a
scope, for example:

```json
{"command":"./update.sh","protect":["src","Cargo.toml"]}
```

Files are saved exactly; directories include their eligible contents. `"."`
protects the whole eligible workspace. Protection does not limit what the
command can change, and there is no scan after it finishes.

Each modifying call returns a receipt with a `ref` such as `rc-…`, the saved
paths, an expiry time and a status: `saved`, `partial` (some paths could not be
saved; reasons are listed) or `none`. Check the status and listed paths before
relying on a restore: `none` means no earlier state was saved. The receipt says
nothing about whether the call then succeeded. A failure to save is reported
but does not block the call.

Startup, reads and unprotected commands never scan or archive your project.
There is no version history, rollback on failure or configurable retention.

## Inspect and restore

You can work with checkpoints in three places, all with the same actions:

- Ask the model; it uses the `checkpoint` tool.
- In the TUI or plain chat, type `/checkpoints` (and `/checkpoints help`).
- From a shell, run `jecode checkpoints`, which needs no provider or network.

```text
jecode checkpoints list
jecode checkpoints inspect rc-ID src/main.rs
jecode checkpoints show rc-ID src/main.rs
jecode checkpoints restore rc-ID src/main.rs --preview --expected latest
jecode checkpoints restore rc-ID src/main.rs --expected TOKEN
```

Replace `rc-ID` with a reference from `list` and `TOKEN` with the `expected`
value printed by `inspect`. Omit paths to inspect or restore everything the call
saved. Use `--` before paths that begin with `-`.

| Action | Result |
| --- | --- |
| `list` | Recent unexpired checkpoints, 20 by default (`--limit` up to 200). |
| `inspect` | The saved and current state of each path, the changes a restore would make and an `expected` token describing the current state. |
| `show` | The saved text of one file. `--offset`, `--byte-offset` and `--limit` page through it; in a conversation it shows one page at a time. |
| `restore` | Puts the selected paths back. |

A restore needs `--expected`: either the token from `inspect`, so the restore
refuses if the files changed since you looked, or `latest` to accept their
current state. `--preview` reports the plan without changing anything.

Before restoring, Jecode verifies the saved content and saves the current state
as a new checkpoint, so every restore can be undone; the result gives the
reference, paths and token to use. Restores happen path by path, not as one
transaction: a partial result lists restored, conflicting, failed and skipped
paths. Binary files can be restored but not shown.

The model can also read a saved file directly with
`{"path":"checkpoint:rc-ID:src/main.rs"}`.

## Scope rules for directories

When a directory is protected, Jecode walks it and skips:

- version-control metadata and Jecode's own storage;
- generated and cache directories such as `target`, `node_modules`, `.venv` and
  `.cache`;
- directories ignored by the project's `.gitignore` files or the repository's
  `info/exclude` (your global Git excludes file is not read).

Only directories are skipped this way: an ignored file such as `.env` inside a
protected directory is still saved. Naming a generated or ignored path
explicitly in `protect` includes it. Symbolic links are saved as links.

A restore can delete files created after the checkpoint only where the saved
scope was complete. Unreadable directories, unsupported entries and Windows
reparse points make a scope incomplete, and the receipt says so.

## Storage and expiry

Checkpoints live in `~/.jecode/checkpoints/<project-name>-<id>/`, one immutable
`rc-ID.rcp` file per call. Each call stores a complete copy of what it saved;
nothing is deduplicated, so repeatedly editing a very large file uses space
quickly. Jecode keeps a 256 MiB free-space margin; when space runs short, a
capture becomes `partial` rather than filling the disk.

Every checkpoint, including the undo saved by a restore, expires 24 hours after
it was created. Using it does not extend that time. While Jecode runs, it deletes
expired checkpoints once a minute; otherwise they are deleted the next time
Jecode starts or `jecode checkpoints` runs. No background service is installed.
Expiry uses the wall clock, so the system time must be correct.

Checkpoints contain unredacted copies of project files, including secrets in
ignored files. Treat the directory as sensitive.

## Limits

- Several Jecode sessions coordinate on the same checkpoints, but other programs
  writing the same files do not. A restore checks current content first; it
  cannot form a transaction with another writer.
- Like `write` and `edit`, checkpoints sync file contents but not directory
  entries, so a power loss right after a call can lose the newest checkpoint.
- If a restore is interrupted, find its undo checkpoint with `list` and inspect
  it before choosing what to restore.
