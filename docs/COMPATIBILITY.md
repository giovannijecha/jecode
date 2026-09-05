# Compatibility

Jecode is still pre-1.0. This document defines the surface frozen for the
`1.0.0` release-candidate cycle and the compatibility promise that begins with
the stable release.

Until `1.0.0`, development is limited to correctness, security, stability,
performance, recovery, accessibility, documentation, and maintenance of the
capabilities already present. New product surface is deferred.

## Version policy

- Pre-1.0 fixes, hardening, polish, and maintenance advance the patch version.
- Release candidates use `1.0.0-rc.N` and the npm `next` channel.
- `1.0.0` starts the compatibility promise and is published on `latest`.
- During 1.x, compatible additions advance the minor version, fixes advance the
  patch version, and intentional breaking changes wait for the next major.

## Stable surface

### Runtime and invocation

Jecode supports Node.js 22.18 or newer on the 22.x line, and Node.js 24 or
newer, on Windows, Ubuntu, and macOS. The supported startup forms are:

```text
jecode [options]
jecode -c [options]
jecode resume [--last] [options]
```

The stable startup options are `--root`, `--reduced-motion`, and `--ephemeral`,
plus `-h`/`--help` and `-v`/`--version`. `-c` is equivalent to `resume --last`;
both resume the newest available conversation in the selected workspace without
a picker.
`--last` is valid only with `resume`. The pre-1.0 `--latest` spelling was
replaced by `--last` and is rejected with guidance to use the new forms.

All conversation launches require a terminal on both stdin and stdout. Other
streams are rejected on stderr with a non-zero exit before configuration,
provider selection, or session work. `--help` and `--version` remain available
through pipes and redirection and finish before those checks. Batch execution
was removed before 1.0.

Provider, model, effort, output limits, and compaction preferences come from
saved settings or built-in defaults and are changed through `/models`, `/effort`,
and `/settings`. Reduced motion retains flag, environment, saved setting, then
default precedence. Ephemeral mode retains flag, environment, then default
precedence and is not saved. The supported preference environment names are
`JECODE_REDUCED_MOTION` and `JECODE_EPHEMERAL`. `NO_COLOR` and provider credential
environment names remain supported.

The pre-1.0 `--provider`, `--model`, `--effort`, `--max-tokens`, `--max-steps`,
`--compaction-percent`, and `--auto-approve` flags are rejected with removal
guidance and the applicable TUI control. Their corresponding `JECODE_PROVIDER`,
`JECODE_MODEL`, `JECODE_EFFORT`, `JECODE_MAX_TOKENS`, `JECODE_MAX_STEPS`,
`JECODE_COMPACTION_PERCENT`, and `JECODE_AUTO_APPROVE` environment variables stop
startup when nonempty; diagnostics identify the variable without echoing its
value. There is no public model-request budget or automatic approval at launch.

### Interactive commands

The stable slash commands are `/help`, `/exit`, `/new`, `/export`,
`/timeline`, `/compact`, `/permissions`, `/settings`, `/effort`, `/models`,
and `/providers`.

Their documented outcomes are stable. Menus, wording, spacing, colours, and
other presentation details may improve without constituting a compatibility
break.

Menus remain available during a model turn. Model and context settings apply
to the next turn; the current turn retains its request and checkpoint identity.
Approvals temporarily take focus and then restore the open menu. Conversation
reset, timeline selection, and manual compaction require a settled turn.

### Providers

The stable provider IDs are:

- `anthropic` for the Anthropic API;
- `openai` for the OpenAI API;
- `ollama` for the official Ollama cloud API, with an API key.

Local models and custom Ollama endpoints were retired before 1.0.
`--ollama-host` is rejected. Legacy `OLLAMA_HOST` and saved `ollamaHost` values
are recognized only for compatibility: official-cloud values do not change
routing and are omitted on the next settings save; other values stop startup
and block settings writes until explicitly removed. No migration silently
redirects a saved local or custom connection to cloud.

Provider-owned model IDs, catalogues, limits, and availability are not frozen
by Jecode. Provider-specific wire formats remain internal.

The `openai-codex` ChatGPT integration remains experimental and outside the
1.x compatibility promise. It is tested and maintained, but its authentication
and upstream transport may need to change independently of the stable provider
surface.

### Controller, tools, and context

The following behavioral guarantees are stable:

- one model-facing controller owns the visible loop;
- interactive guidance enters the active turn at a safe provider/tool boundary
  without replaying already-issued work or creating another controller;
- ordinary turns have no arbitrary model-request ceiling;
- historical tool calls are never replayed during resume or branching;
- dangerous tools ask by default and require the applicable approval or session
  permission; session policies remain mutable and `/new` resets them;
- interruption settles or rolls back foreground work at its documented commit
  boundary;
- automatic and manual compaction change only model-facing context, not the
  canonical conversation, transcript, export, or tree.

Tool schemas may gain optional fields during 1.x. Removing a tool, renaming a
required field, or weakening its workspace and permission boundary is a
breaking change.

### Persistent data

User data remains under `~/.jecode`:

- `settings.json` stores non-secret defaults;
- `credentials.json` stores explicitly saved API keys;
- `accounts.json` stores ChatGPT OAuth accounts;
- `sessions/` stores workspace-scoped durable conversations.

The TUI-only startup change does not alter saved settings or session schemas
and requires no data migration.

These files are implementation-owned. Use Jecode's controls for ordinary changes;
manual edits are reserved for explicit migration or recovery instructions, such
as removing the retired `ollamaHost` field described above. Their exact JSON
layout is not a public API. Jecode 1.x will continue to read session
schemas 1, 2, 3, and 4. A future schema change must migrate safely or fail
without destroying the existing session. Bounded catalogue summaries are
rebuildable indexes, not canonical conversation data; missing or suspect
summaries fall back to strict session loading. Version 1 catalogue indexes are
rebuilt on the next idle listing to recover checkpoints that older writers
could leave hidden; canonical conversation schemas and node files do not change.

`JECODE_HOME` is reserved for development and test isolation. It is not a
supported user-facing configuration surface and is not covered by the 1.x
compatibility promise.

A conversation accepts at most 1,024 durable nodes. At the limit, a further
turn is rejected with guidance to start `/new`; existing data remains intact.
The resume catalogue refuses to scan a workspace store containing more than
4,096 entries and never deletes sessions automatically.

### Terminal interaction

The interactive UI requires a TTY on both stdin and stdout with UTF-8 text,
ANSI cursor control, an alternate screen, and bracketed paste support.
`NO_COLOR` disables semantic colour, and `--reduced-motion` avoids tool-state
and evidence animations and keeps the input cursor steady. Provider text
continues to stream with either accessibility setting.

Keyboard actions documented in the [user guide](USAGE.md#keyboard-controls)
remain supported. Exact glyphs, palette values, line wrapping, and component
layout may change as long as the same information, controls, accessibility
modes, and frame safety remain.

## Internal surface

Unexported modules, provider request bodies, cache formats, workspace digests,
atomic-write mechanics, rendering algorithms, diagnostic wording, and ignored
build output are internal. They may change in any compatible release to improve
correctness, safety, or performance.

## Release-candidate gate

`1.0.0-rc.1` is eligible only after the complete automated matrix passes, the
stable providers and experimental ChatGPT integration have been exercised with
authorized live accounts, physical terminal checks cover the supported
platforms, and no confirmed critical, high, or medium finding remains open.

Before promoting a candidate to stable `1.0.0`, complete an explicit
release-candidate soak against one immutable candidate version and commit:

- Observe at least seven consecutive calendar days and ten real work sessions
  across at least two workspaces. Include three sessions of at least one hour
  and thirty completed turns each; idle time alone is not soak evidence.
- Exercise every stable provider in at least two sessions and complete a live
  sign-in and conversation through the experimental ChatGPT integration. This
  does not promote that integration into the stable compatibility promise.
- Complete the terminal, recovery, security-boundary, and performance checks in
  the [validation protocol](../dev/validation/README.md). Record the environment,
  outcome, and evidence for each required check; a missing result is not a pass.
- Close all confirmed critical, high, or medium findings and confirmed candidate
  regressions. Every fix needs a focused regression check. A replacement
  candidate repeats the automated matrix and restarts the soak.

The stable promotion may change version and release metadata only. Runtime or
shipped-interface changes require another candidate. The promotion commit still
needs review, the complete CI matrix, and package verification before tagging.
Keep the reviewed validation record linked from the promotion pull request.
