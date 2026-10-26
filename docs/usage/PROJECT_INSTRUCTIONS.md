# Project instructions

Create `AGENTS.md` to supply project rules, such as coding conventions, required
checks and files to preserve. This Markdown format can be shared with other coding
agents. Jecode loads these files from the launch directory and its subdirectories
before each new turn in the TUI, plain chat, single prompts and piped tasks. No
setting is needed.

For example:

```markdown
# Project rules

- Keep changes focused on the requested behavior.
- Preserve existing public interfaces unless the task explicitly changes them.
- Run the project's relevant checks before reporting a change as complete.
- Write code, comments and documentation in English.
```

## Initialize project rules

In the TUI or `--plain` chat, run `/init` to ask the current model to inspect the
project and create or update the launch directory's `AGENTS.md`. It uses a normal
agent turn and the existing file tools, so normal model usage applies. The task
asks for concise, verified conventions and build/test commands, preserving useful
existing instructions through targeted updates.

In the TUI, `/init` waits in the command queue while a turn is running. Stopping
or failing that turn pauses it like other pending drafts. Review it in `/drafts`
before sending again. The generated instructions become active on the next turn.

## Scope and precedence

Only files named `AGENTS.md` are discovered. Use the uppercase name exactly on
case-sensitive filesystems. Parent directories and global locations are never
searched. Other filenames, including `JECODE.md`, are not automatically loaded;
`@file` references have no special expansion syntax.

If a project previously used `JECODE.md`, move its relevant instructions into
`AGENTS.md`, preserving any useful existing content. Jecode does not rename or
merge instruction files automatically.

Each file applies to the directory containing it and that directory's
descendants. For example, when launched in `project/`:

| Instruction file | Applies to |
| --- | --- |
| `AGENTS.md` | The whole project. |
| `web/AGENTS.md` | `web/` and its descendants. |
| `api/AGENTS.md` | `api/` and its descendants. |

More specific instructions take precedence within their scope. Explicit user
requests take precedence over project instructions, and native tool contracts
remain in force. Each loaded section identifies its source and scope for the
model; rules from sibling directories do not override each other.

Launching in `project/web/` excludes `project/AGENTS.md`. A Bash tool changing its
own directory does not move this boundary. Discovery skips subdirectories named
`.git`, `node_modules`, `.venv` or `vendor` at every level and does not traverse
directory symbolic links or junctions. Other real subdirectories, including
`target`, are searched even if ignored by Git, so local instruction files remain
usable. An explicitly selected launch directory is always included, even if it
has one of those names.

## Updates and saved sessions

Each new turn starts with one snapshot of all discovered files. Ordinary edits,
creation or deletion of `AGENTS.md` take effect on the next turn, including when
tools change the files during a running turn. Follow-up tool calls and live
requests after context compaction keep the same snapshot.
After `/resume`, the next turn reads the current files from the launch directory
and its subdirectories.

Restoring an instruction file through this conversation's checkpoints refreshes
the instructions before the next model request, including within the same turn.

Instructions enter the current model context without rewriting saved messages
or the exported system message. Compaction retains the turn's snapshot; see
[LONG_WORK.md](LONG_WORK.md).

## File requirements

`AGENTS.md` is optional, including at the launch root. A missing, empty or
whitespace-only file adds no rules. Use regular UTF-8 text files; a UTF-8
byte-order mark is accepted. Each file and the combined source text have a 64 KiB
limit. A file symbolic link must resolve inside the launch directory.

An unreadable instruction file or directory, invalid UTF-8, a non-file instruction
path, an out-of-scope file link or an exceeded size limit stops the new request
before contacting the model. Fix or remove the cause and submit the request
again. Instructions are never silently truncated.
The credential redactor masks the configured OpenRouter key and saved ChatGPT
tokens before project instructions enter a model request.
