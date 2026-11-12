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
not supported. A directory whose name breaks the `name` rule, such as one containing
a space or a dot, and plain files directly under `skills/` are ignored without a
warning. Any other invalid skill is skipped with a warning shown in `/skills` and to
the model; the other skills still work. Jecode inspects the first 128 entries in name
order and stops adding skills once catalog metadata reaches 48 KiB, with a warning in
both cases. The frontmatter must close within the first 8 KiB of `SKILL.md`.

Keep longer reference material in separate files inside the skill directory and
link to them from `SKILL.md`. Copying the whole directory keeps those links
working. Skills written for other agents may need changes to match Jecode's
tools.

## How the model uses skills

The model sees your skills' names and descriptions in the instruction snapshot
shared with [project instructions](PROJECT_INSTRUCTIONS.md#when-changes-take-effect);
a changed list is sent once, when it changes. When one fits, the model
reads it, and any supporting files it needs, with references such as
`skill:debugging` or `skill:debugging/references/DETAILS.md`. Skill bodies are
never sent on every request. There is no per-skill slash command.

## Ask for a skill

To ask for a skill yourself, name it in your message as `$name`, such as
`$debugging the login test fails`. In the TUI, type `$` at the start of a word to
choose from a **Skills** sheet that lists each skill with its description and
filters as you type; Enter or Tab writes `$name` into your draft without sending
it, and Esc keeps a literal `$`. See
[Terminal interface](INTERFACE.md#reference-skills-and-files). In plain chat or a
single prompt, type `$name` directly.

The skill catalog the model receives says that `$name` is an explicit request
for that skill. The model then reads `skill:name` and applies it to the
request; Jecode does not load the body in advance or check that it was used.
The selector reads the catalog from `~/.jecode/skills/` each time it opens,
locally and without calling the model: an empty catalog, or one whose skills
were all skipped, says so, and `/skills` explains skipped entries. A `$` word
that names no skill in the catalog is ordinary text.

The list is refreshed at the start of each turn, whenever `/skills` or the skill
selector opens, and when a new message joins a running turn.
Within a turn, the first read of each file stays fixed; the next turn reads the
files from disk again. Skills support your current request; they do not override
your instructions or grant permission for extra work. After compaction, the
model is reminded of skill references retained from successful read requests.
This includes references in saved batch results whose text it may not have
opened yet; the model can read or reread the resource when relevant.

Skill references are read-only and cannot reach outside their skill directory.
Each file can be up to 2 MiB, with 8 MiB of skill content per turn. A read also
returns the skill's directory path, so the model can locate scripts.

Reading a skill never runs anything. The model can run a skill's script through
`bash` or `exec` when appropriate, subject to your permission level and the
script's own requirements. Relative paths inside a skill refer to the skill
directory, not the project.

In the TUI, a skill read appears as a **Skill** card with the skill's name;
Ctrl+O shows the text it returned.
