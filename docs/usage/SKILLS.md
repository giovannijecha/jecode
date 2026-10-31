# Personal skills

A skill is a reusable procedure you write once and the model uses when it fits
the task, such as how you like bugs investigated or releases prepared. Skills
are personal: they live in your Jecode home and apply to every project. Project
rules belong in [`AGENTS.md`](PROJECT_INSTRUCTIONS.md).

## Add a skill

Each skill is a directory under `~/.jecode/skills/` (`%USERPROFILE%\.jecode\skills\`
on Windows) containing a `SKILL.md` and, optionally, supporting files. Create
`~/.jecode/skills/debugging/SKILL.md`:

```markdown
---
name: debugging
description: Investigate an observed failure, identify its cause and verify a focused fix.
---

# Investigate a failure

Reproduce the reported behavior before changing code. Trace the failing path,
compare it with the expected behavior, fix the cause and run relevant checks.
Report any verification that could not be run.
```

Then run `/skills` in the TUI or plain chat. It lists the skills Jecode found,
with warnings for invalid ones, without calling the model.

The frontmatter needs two fields:

| Field | Rules |
| --- | --- |
| `name` | 1–64 ASCII letters, digits, hyphens or underscores, matching the directory name. |
| `description` | 1–512 bytes on one line. Say when the skill applies: it is all the model sees before choosing to read the skill. |

Values can be plain or wrapped in matching single or double quotes. This is a
small frontmatter format, not YAML: multi-line values and escape sequences are
not supported. An invalid skill is skipped with a warning shown in `/skills` and
to the model; the other skills still work.

Keep longer reference material in separate files inside the skill directory and
link to them from `SKILL.md`. Copying the whole directory keeps those links
working. Skills written for other agents may need changes to match Jecode's
tools.

## How the model uses skills

Each request lists your skills' names and descriptions. When one fits, the model
reads it, and any supporting files it needs, with references such as
`skill:debugging` or `skill:debugging/references/DETAILS.md`. Skill bodies are
never sent on every request. You can also ask for a skill by name in your
message; there is no per-skill slash command.

The list is refreshed at the start of each turn and whenever `/skills` runs.
Within a turn, the first read of each file stays fixed; the next turn reads the
files from disk again. Skills support your current request; they do not override
your instructions or grant permission for extra work. After compaction, the
model is reminded which skills it had read so it can read them again.

Skill references are read-only and cannot reach outside their skill directory.
Each file can be up to 2 MiB, with 8 MiB of skill content per turn. A read also
returns the skill's directory path, so the model can locate scripts.

Reading a skill never runs anything. The model can run a skill's script through
`bash` or `exec` when appropriate, subject to your permission level and the
script's own requirements. Relative paths inside a skill refer to the skill
directory, not the project.

In the TUI, a skill read appears as a **Skill** card with the skill's name;
Ctrl+O shows the text it returned.
