# Project instructions

Put project rules in an `AGENTS.md` file: coding conventions, required checks,
files to leave alone. Jecode reads these files before every new turn, in the
TUI, plain chat, single prompts and pipes. Other coding agents use the same
format, so one file can serve all of them.

```markdown
# Project rules

- Keep changes focused on the requested behavior.
- Preserve public interfaces unless the task changes them.
- Run `cargo test` before reporting a change as complete.
- Write code, comments and documentation in English.
```

## Create one with /init

In the TUI or `--plain`, run `/init`. The current model inspects the project and
creates or updates the `AGENTS.md` in the launch directory, keeping useful
existing content and adding only conventions and commands it can verify. It is a
normal agent turn, so it uses your model and permission level as usual. If a
turn is running, `/init` waits until it ends. The new instructions apply from
the next turn.

## Which files apply

Jecode looks for files named exactly `AGENTS.md` in the directory where you
started it and in all its subdirectories. Each file applies to its own directory
and everything below it. Launched in `project/`:

| File | Applies to |
| --- | --- |
| `AGENTS.md` | The whole project. |
| `web/AGENTS.md` | `web/` and below. |
| `api/AGENTS.md` | `api/` and below. |

More specific files take precedence within their directory; sibling directories
do not affect each other. Your explicit requests take precedence over project
instructions, and Jecode's tool rules always apply.

- Parent directories are never read. Launched in `project/web/`,
  `project/AGENTS.md` does not apply. A command changing directory does not move
  this boundary.
- The search skips `.git`, `node_modules`, `.venv` and `vendor` directories and
  does not follow directory links or junctions. Other directories, including
  `target`, are searched even when Git ignores them.
- Other names, such as `JECODE.md`, are not read, and `@file` references are not
  expanded. If you used `JECODE.md` before, move its content into `AGENTS.md`.

## When changes take effect

Each new turn takes one snapshot of all instruction files, and the snapshot
stays fixed for the rest of the turn, including after compaction. Edits,
additions and deletions, even ones made by the agent during a turn, apply from
the next turn. Restoring an `AGENTS.md` from a checkpoint applies before the next
model request. After `/resume`, the next turn reads the current files.

Instructions enter the model's context without changing the saved conversation.

## Requirements and errors

`AGENTS.md` is optional; a missing or empty file adds nothing. Files must be
UTF-8 text (a byte-order mark is accepted) of at most 64 KiB each, and all of
them together at most 64 KiB. A linked `AGENTS.md` must point inside the launch
directory.

An unreadable file, invalid UTF-8, an instruction path that is not a file, a
link pointing outside the launch directory or an exceeded size limit stops the
request before anything is sent. Fix or remove the cause and send the request
again. Instructions are never silently truncated. Known credentials are masked
before instructions reach the model.
