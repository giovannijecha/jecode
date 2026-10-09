# Project instructions

Create `JECODE.md` in the directory where you run Jecode to supply project rules,
such as coding conventions, required checks and files to preserve. Jecode loads
it automatically before each new user request in the TUI, plain chat, single
prompts and piped tasks. No command or configuration setting is required.

For example:

```markdown
# Project rules

- Keep changes focused on the requested behavior.
- Preserve existing public interfaces unless the task explicitly changes them.
- Run the project's relevant checks before reporting a change as complete.
- Write code, comments and documentation in English.
```

## Scope and precedence

Only the launch directory's `JECODE.md` is loaded. Jecode does not search parent
or child directories, load a personal global file, or import other instruction
files. Use the uppercase name exactly on case-sensitive filesystems.
The launch directory remains the scope even when a Bash command changes its
own working directory.

The file provides project instructions to the model. Explicit user requests
take precedence over those instructions, and the native tool contracts remain
in force.

## Updates and saved sessions

Each submitted request uses one snapshot of the file for its entire turn,
including follow-up tool calls and the live requests after context compaction.
Edits, creation or deletion of `JECODE.md` take effect on the next user request.
After `/resume`, the next request reads the current file from the launch
directory. A running turn keeps its snapshot even if a tool edits the file.

Instructions are inserted into the current model context separately from saved
original messages. Loading the file does not rewrite the conversation or copy
its contents into the original history or exported system message. Context
compaction retains the snapshot directly rather than summarizing its rules.

## File requirements

`JECODE.md` is optional. A missing, empty or whitespace-only file adds no rules.
Use a regular UTF-8 text file of at most 64 KiB; a UTF-8 byte-order mark is
accepted. A symbolic link must resolve inside the launch directory.

An unreadable file, invalid UTF-8, a non-file path or an oversized file stops
the new request with an error before contacting the model. Fix or remove the
file and submit the request again. Instructions are never silently truncated.
The configured OpenRouter key is masked by the existing credential redactor.

For this repository, `JECODE.md` is a local instruction file excluded from Git.
