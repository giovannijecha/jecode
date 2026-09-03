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
jecode resume [--latest] [options]
```

The stable options are `--root`, `--provider`, `--model`, `--ollama-host`,
`--effort`, `--max-tokens`, `--max-steps`, `--compaction-percent`,
`--reduced-motion`, `--auto-approve`, and `--ephemeral`, plus `-h`/`--help`
and `-v`/`--version`. `--latest` is valid only with `resume`.

Persistent configuration precedence remains command-line flag, environment
variable, saved setting, then built-in default. `--max-steps` and
`JECODE_MAX_STEPS` are an opt-in process budget: no limit applies by default,
and no value is stored in settings. The stable environment names are
`JECODE_PROVIDER`, `JECODE_MODEL`, `OLLAMA_HOST`, `JECODE_EFFORT`,
`JECODE_MAX_TOKENS`, `JECODE_MAX_STEPS`, `JECODE_COMPACTION_PERCENT`,
`JECODE_REDUCED_MOTION`, `JECODE_AUTO_APPROVE`, and `JECODE_EPHEMERAL`.
Provider credentials retain their documented environment names.

### Interactive commands

The stable slash commands are `/help`, `/exit`, `/new`, `/export`,
`/timeline`, `/compact`, `/permissions`, `/settings`, `/effort`, `/models`,
and `/providers`.

Their documented outcomes are stable. Menus, wording, spacing, colours, and
other presentation details may improve without constituting a compatibility
break.

### Providers

The stable provider IDs are:

- `anthropic` for the Anthropic API;
- `openai` for the OpenAI API;
- `ollama` for local, cloud, or explicitly configured compatible endpoints.

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
- ordinary turns have no arbitrary model-request ceiling; an explicit
  `--max-steps` budget can constrain deterministic automation;
- historical tool calls are never replayed during resume or branching;
- dangerous tools require the applicable permission unless launch policy
  explicitly allows them;
- interruption settles or rolls back foreground work at its documented commit
  boundary;
- automatic and manual compaction change only model-facing context, not the
  canonical conversation, transcript, export, or tree;
- batch mode remains stateless, line-oriented, and non-zero on terminal
  failure.

Tool schemas may gain optional fields during 1.x. Removing a tool, renaming a
required field, or weakening its workspace and permission boundary is a
breaking change.

### Persistent data

User data remains under `~/.jecode`:

- `settings.json` stores non-secret defaults;
- `credentials.json` stores explicitly saved API keys;
- `accounts.json` stores ChatGPT OAuth accounts;
- `sessions/` stores workspace-scoped durable conversations.

These files are implementation-owned and should not be edited manually. Their
exact JSON layout is not a public API. Jecode 1.x will continue to read session
schemas 1, 2, 3, and 4. A future schema change must migrate safely or fail
without destroying the existing session.

`JECODE_HOME` is reserved for development and test isolation. It is not a
supported user-facing configuration surface and is not covered by the 1.x
compatibility promise.

A conversation accepts at most 1,024 durable nodes. At the limit, a further
turn is rejected with guidance to start `/new`; existing data remains intact.
The resume catalogue refuses to scan a workspace store containing more than
4,096 entries and never deletes sessions automatically.

### Terminal interaction

The interactive UI requires a TTY with UTF-8 text, ANSI cursor control, an
alternate screen, and bracketed paste support. Non-interactive streams use
batch mode. `NO_COLOR` disables semantic colour, and `--reduced-motion` avoids
the blinking cursor used for active work.

Keyboard actions documented in the README remain supported. Exact glyphs,
palette values, line wrapping, and component layout may change as long as the
same information, controls, accessibility modes, and frame safety remain.

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
