# Personal skills

Skills are reusable workflows stored in `~/.jecode/skills/` (on Windows,
`%USERPROFILE%\.jecode\skills\`). `JECODE_HOME` moves this library together
with the rest of the personal configuration. Each skill has its own directory
containing `SKILL.md` and, optionally, supporting files.

Use `/skills` in the TUI or plain chat to list available names and descriptions.
The command is local: it does not call the model or activate a skill.

## Add a skill

Create `~/.jecode/skills/debugging/SKILL.md` with this structure:

```markdown
---
name: debugging
description: Investigate an observed failure, identify its cause and verify a focused fix.
---

# Investigate a failure

Reproduce the reported behavior before changing code. Trace the failing path,
compare it with the expected behavior, fix the cause and run relevant checks.
Preserve unrelated work and report any verification that could not be run.
```

Choose a descriptive name and explain when the skill applies in its description.
The name must match its directory: 1 to 64 ASCII letters, digits, hyphens or
underscores. The description must contain 1 to 512 UTF-8 bytes on one line.
Both values can be plain text or enclosed in matching single or double quotes.
This is a small frontmatter format; YAML blocks and escape processing are not
supported. Invalid entries are skipped with a warning visible in `/skills` and
the model's catalog, while valid siblings remain available.

The body contains the procedure. Keep substantial reference material in separate
files and link to it from `SKILL.md`. Copying a complete skill directory preserves
those relative references. The library is personal; project rules belong in
[`JECODE.md`](PROJECT_INSTRUCTIONS.md).

Jecode supplies the skill format and loader. Personal skill files are installed
separately and can be edited without rebuilding the application. A skill copied
from another agent may need changes for Jecode's available tools and environment.

## How skills enter the conversation

The model receives the library's names, descriptions and read references. It
chooses relevant skills for the task and loads their bodies through `read`, rather
than receiving every procedure on every request. You can also ask it in ordinary
text to use a named skill. There is no per-skill slash command or composer expansion.
Jecode continues to work when the library is empty.

The catalog is refreshed for a new turn and when `/skills` lists it. Skill
procedures support the current request; they do not override your instructions
or grant permission for additional work.

Read references from earlier skill use remain identifiable after compaction and
resume, so the model can reread the current files when relevant. They do not
automatically apply an old workflow to a new request. Each resource's first read
is kept stable for the rest of that turn; a later turn reads from disk again.

The TUI presents a skill load as `● Skill(name)`, using the existing tool-card
states: green after success, red after failure and the tool accent while running.
Expand the latest finished turn with `Ctrl+O` to inspect the returned text.
Supporting files use ordinary Read cards.

## References and scripts

| Read reference | Content |
| --- | --- |
| `skill:debugging` | The skill's `SKILL.md`. |
| `skill:debugging/references/DETAILS.md` | A supporting file inside that skill. |

Skill reads use the same paging arguments as other [text reads](TOOLS.md#file-paths-and-paged-reads).
The references are read-only and resolve within the selected skill directory;
they do not expose the rest of the personal configuration.
Reads also return `package_path`, the skill's directory, for resolving supporting
files and script paths. Text resources are limited to 2 MiB each, with an 8 MiB
snapshot budget per turn.

Loading a skill does not run scripts. The model can use the existing Bash tool
to run a supporting script when it is appropriate to the task and its required
system tools are installed. Relative resources belong to the skill directory,
not the project working directory.
