# Project instructions

Create `JECODE.md` to supply project rules, such as coding conventions, required
checks and files to preserve. Jecode loads these files from the launch directory
and its subdirectories before each new turn in the TUI, plain chat, single
prompts and piped tasks. No setting is needed.

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
project and create or update the launch directory's `JECODE.md`. It uses a normal
agent turn and the existing file tools, so normal model usage applies. The task
asks for concise, verified conventions and build/test commands, preserving useful
existing instructions through targeted updates.

In the TUI, `/init` waits in the command queue while a turn is running. Stopping
or failing that turn pauses it like other pending drafts. Review it in `/drafts`
before sending again. The generated instructions become active on the next turn.

## Scope and precedence

Only files named `JECODE.md` are discovered. Use the uppercase name exactly on
case-sensitive filesystems. Parent directories and global locations are never
searched. Other filenames, including `AGENTS.md`, are not automatically loaded;
`@file` references have no special expansion syntax.

Each file applies to the directory containing it and that directory's
descendants. For example, when launched in `project/`:

| Instruction file | Applies to |
| --- | --- |
| `JECODE.md` | The whole project. |
| `web/JECODE.md` | `web/` and its descendants. |
| `api/JECODE.md` | `api/` and its descendants. |

More specific instructions take precedence within their scope. Explicit user
requests take precedence over project instructions, and native tool contracts
remain in force. Each loaded section identifies its source and scope for the
model; rules from sibling directories do not override each other.

Launching in `project/web/` excludes `project/JECODE.md`. A Bash tool changing its
own directory does not move this boundary. Discovery skips `.git` metadata and
does not traverse directory symbolic links or junctions. Other real subdirectories
are searched even if ignored by Git, so local instruction files remain usable.

## Updates and saved sessions

Each new turn uses one snapshot of all discovered files for its entire duration,
including follow-up tool calls and the live requests after context compaction.
Edits, creation or deletion of `JECODE.md` take effect on the next turn.
After `/resume`, the next turn reads the current files from the launch directory
and its subdirectories. A running turn keeps its snapshot even if a tool edits
one of those files; follow-up messages joining that turn use the same snapshot.

Instructions enter the current model context without rewriting saved messages
or the exported system message. Compaction retains the turn's snapshot; see
[LONG_WORK.md](LONG_WORK.md).

## File requirements

`JECODE.md` is optional, including at the launch root. A missing, empty or
whitespace-only file adds no rules. Use regular UTF-8 text files; a UTF-8
byte-order mark is accepted. Each file and the combined source text have a 64 KiB
limit. A file symbolic link must resolve inside the launch directory.

An unreadable instruction file or directory, invalid UTF-8, a non-file instruction
path, an out-of-scope file link or an exceeded size limit stops the new request
before contacting the model. Fix or remove the cause and submit the request
again. Instructions are never silently truncated.
The configured OpenRouter key is masked by the existing credential redactor.
