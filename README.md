<p align="center">
  <img src="docs/assets/brand/jeco-256.png" width="128" alt="Jeco, the steel-blue Jecode gecko">
</p>

<p align="center">
  <img src="docs/assets/brand/wordmark-steel.svg" width="280" alt="Jecode">
</p>

<p align="center"><strong>Your code. Your loop.</strong></p>

<p align="center">
  A focused terminal coding agent with visible tool use, durable sessions, and
  one controller that stays under your control.
</p>

<p align="center">
  <a href="https://github.com/giovannijecha/jecode/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/giovannijecha/jecode/actions/workflows/ci.yml/badge.svg"></a>
  <a href="LICENSE"><img alt="MIT License" src="https://img.shields.io/badge/license-MIT-669BD2"></a>
  <img alt="Node.js 22.18+ or 24+" src="https://img.shields.io/badge/node-22.18%2B%20%7C%2024%2B-86CB92">
  <img alt="Runtime dependencies: zero" src="https://img.shields.io/badge/runtime_dependencies-0-8DB4DD">
</p>

<p align="center">
  <a href="https://github.com/giovannijecha/jecode/blob/main/CHANGELOG.md">Changelog</a> &middot;
  <a href="https://github.com/giovannijecha/jecode/blob/main/docs/compatibility.md">Compatibility</a> &middot;
  <a href="https://github.com/giovannijecha/jecode/releases">Releases</a>
</p>

> Jecode is currently pre-1.0. The core loop is usable today, but commands and
> terminal interactions may still evolve before 1.0. The release-candidate
> surface is now frozen in the [compatibility contract](https://github.com/giovannijecha/jecode/blob/main/docs/compatibility.md).

## Why Jecode

- **One visible loop.** One controller talks to the model, runs tools, and
  returns control to you. Independent reads may overlap inside a step; writes
  and commands stay ordered. You can steer an active turn without starting a
  second loop. There are no delegated agents or hidden model workers.
- **Terminal-native today.** The transcript, composer, searchable menus, tool
  output, diffs, approvals, reasoning, and status share one full-screen TUI.
- **Permission-aware.** Tool use is visible. Dangerous actions ask first, and
  remembered session approvals can be reviewed or revoked.
- **Durable and context-bounded.** Interactive conversations survive terminal
  exits. Older model context can be compacted while the complete conversation
  and transcript remain available in the saved session.
- **Multi-provider.** Use Anthropic or OpenAI API keys, an eligible ChatGPT
  account, or a local, cloud, or remote Ollama server without changing the
  workflow.
- **Lean by construction.** Jecode ships as owned JavaScript built on Node.js
  primitives, executes no installation scripts, and has zero third-party
  runtime dependencies. This is a permanent product constraint, not a temporary
  optimization.

## Quick start

Jecode requires **Node.js 22.18+ on the 22.x line, or Node.js 24+**, and npm.

### Install or update

Install the current stable release. Running the same command again updates an
existing installation:

```console
npm install --global @giovannijecha/jecode@latest
```

Confirm the installed version:

```console
jecode --version
```

### Start Jecode

Open the project you want Jecode to work on:

```console
cd path/to/your/project
```

Then start Jecode:

```console
jecode
```

Jecode opens directly on an empty composer. Use `/providers` to connect a
service, `/models` to choose a model, then describe the work you want done.

```text
Review this project, explain its architecture, and propose the smallest safe
change to improve startup performance.
```

Use `jecode --root path/to/project` to select another workspace, or
`jecode --ephemeral` when the conversation must stay memory-only.

### Resume a conversation

Open the searchable resume picker for the current workspace:

```console
jecode resume
```

Resume the most recently updated conversation directly:

```console
jecode resume --latest
```

List every startup option:

```console
jecode --help
```

Windows, Ubuntu, and macOS are covered by the project test matrix.

### Prereleases

Prereleases exist only during an announced release-candidate cycle. When one is
active, its GitHub release provides the exact installation command. Outside an
active cycle, the stable npm package is the only supported installation
artifact.

Git URL installs are intentionally unsupported: the source tree contains no
generated runtime and defines no install-time build hook.

### Uninstall

```console
npm uninstall --global @giovannijecha/jecode
```

Uninstalling the command preserves `~/.jecode`. Remove that directory only when
you intentionally want to erase saved settings, credentials, accounts, and
sessions.

### Replace a legacy installation

If an older GitHub installation still owns the `jecode` executable, first
remove the legacy unscoped package:

```console
npm uninstall --global jecode
```

Then install the current scoped package:

```console
npm install --global @giovannijecha/jecode@latest
```

Do not work around the resulting `EEXIST` error with `--force`.

### Linux and WSL

WSL uses its own Node.js installation and `PATH`; the Node.js version installed
on Windows does not apply inside it. Keep user-installed npm commands in the
Linux user path. Set the user-level npm prefix:

```console
npm config set prefix "$HOME/.local"
```

Add it to the current shell's `PATH`:

```console
export PATH="$HOME/.local/bin:$PATH"
```

Install or update Jecode:

```console
npm install --global @giovannijecha/jecode@latest
```

Refresh the command cache:

```console
hash -r
```

Confirm which executable will run:

```console
command -v jecode
```

Verify the installed version:

```console
jecode --version
```

Persist the `PATH` export in `~/.profile` or your shell's startup file. Inside
WSL, `command -v jecode` should resolve below `/home/...`, not through an
inherited Windows path below `/mnt/c`. Do not ignore `EBADENGINE`: `node
--version` must report 22.18+ on the 22.x line, or 24+.

## Providers

| Provider ID | Authentication | Notes |
| --- | --- | --- |
| `anthropic` | `ANTHROPIC_API_KEY` | Anthropic API |
| `openai` | `OPENAI_API_KEY` | OpenAI API |
| `openai-codex` | ChatGPT OAuth | Experimental; uses eligible ChatGPT Codex access |
| `ollama` | `OLLAMA_API_KEY` for cloud or remote use | Cloud with a key, local without one |

Choose **ChatGPT** in `/providers` to sign in on OpenAI's website without
pasting a key. Jecode supports a local browser callback and a device-code flow;
WSL and remote terminals default to the device code. Availability and usage
limits depend on the ChatGPT account and plan, not on OpenAI API credits. This
integration is experimental and is not an endorsement of Jecode by OpenAI.

Anthropic remains API-key only. Jecode does not reuse a Claude consumer
subscription or copy credentials from another client.

For Ollama, `/providers` can select cloud, local, or a custom endpoint. With an
Ollama API key Jecode defaults to `https://ollama.com`; without one it defaults
to `http://127.0.0.1:11434`. Remote custom endpoints must use HTTPS.

## Use the TUI

Type `/` to open searchable command completion inside the composer.

| Command | What it does |
| --- | --- |
| `/settings` | Manage the selected model and saved non-secret defaults |
| `/effort` | Change and save reasoning effort directly |
| `/providers` | Manage provider connections, API keys, ChatGPT sign-in, and Ollama endpoints |
| `/models` | Search all currently usable provider catalogues and select a model |
| `/permissions` | Change session tool access and review remembered approvals |
| `/timeline` | Browse resumable turns and select where the next branch should begin |
| `/compact` | Compact the current branch context without deleting saved conversation history |
| `/new` | Start a new conversation and reset session tool permissions |
| `/export` | Save a timestamped Markdown transcript in the launch directory |
| `/help` | Open a temporary keyboard reference in the composer dock |
| `/exit` | Restore the terminal and exit |

Useful controls:

- **Up/Down** moves through suggestions, menus, and input history.
- **Left/Right** moves the cursor or changes an inline value;
  **Ctrl+Left/Right** moves by word.
- **Backspace/Delete** removes one character;
  **Ctrl+Backspace/Delete** removes one word.
- **Home/End** moves to the beginning or end of the composer.
- **Tab** completes a slash command without running it; **Enter** sends. During
  an active model turn, **Enter** queues guidance for its next safe boundary.
- **Alt+Enter** inserts a newline.
- **Esc** closes the active menu or interrupts foreground work.
- **Ctrl+C** interrupts, or exits while idle. **Ctrl+D** requests a clean exit.
- **PageUp/PageDown** and the mouse wheel scroll the transcript.
- **Ctrl+O** expands or compacts the latest reasoning or tool-detail block.

The footer keeps the active model, effort, and workspace visible. During work,
it adds the current state, elapsed time, steering availability or queue count,
and interrupt hint. Queued guidance joins the same conversation turn after the
provider response or complete tool batch already in progress; `Esc` remains an
immediate interruption. Operational feedback uses the same replaceable status
area instead of adding noise to the conversation or its Markdown export.

## Sessions and context

Interactive conversations are stored under `~/.jecode/sessions` and scoped to
the canonical workspace path. A fresh or `/new` conversation is not added to
the resume picker until it has a settled turn. Resuming and continuing a
conversation keeps its durable session identity and updates one picker entry
instead of creating duplicates.

`/timeline` shows completed, failed, and interrupted turns in the conversation
tree. Selecting an older turn changes only the in-memory path: it creates and
saves a branch only after the next real user message. Cancelling the picker or
exiting before that message leaves the durable head unchanged. A failed turn
keeps the same partial evidence and outcome in the live transcript, export, and
resume, while the next model receives a neutral failure boundary instead of
incomplete streamed text. Historical tools are displayed but never executed.
If a process stops abruptly inside a tool loop, Jecode resumes from the latest
safe ancestor and lets the next user turn create a branch. `/export` writes
only the currently selected path.

When model-facing context approaches the selected model's usable capacity,
Jecode asks the provider for a bounded summary of the older prefix and keeps
recent turns exact. The default trigger is 85% and can be changed from 50% to
95%. Live provider metadata or Ollama's allocated context determines the budget
when available; provider safety limits always win.

Compaction changes only the projection sent to the model. Complete messages,
tool evidence, transcript, export, and conversation tree remain intact. The
branch-local summary anchor is saved with the session so resume does not repeat
the same compaction. `/compact` requests this process immediately, even below
the automatic threshold; it leaves very small contexts unchanged. After
selecting a historical turn, send the first new message before compacting so
shared history is never rewritten.

## Batch mode

When stdin or stdout is not a terminal, Jecode switches to a plain
line-oriented mode:

```console
printf "explain this project\n" | jecode --root .
```

Batch conversations are stateless. Dangerous tools remain denied unless
`--auto-approve` is supplied explicitly. Terminal failures are written to
stderr and exit non-zero so scripts and CI pipelines can stop reliably.

## Configuration

Startup precedence is: command-line flags, environment variables, saved
settings, then built-in defaults.

| Flag | Environment | Default |
| --- | --- | --- |
| `--provider` | `JECODE_PROVIDER` | `anthropic` |
| `--model` | `JECODE_MODEL` | Provider default or interactive selection |
| `--ollama-host` | `OLLAMA_HOST` | Cloud with an Ollama key, local without one |
| `--root` | - | Current directory |
| `--effort` | `JECODE_EFFORT` | `high` |
| `--max-tokens` | `JECODE_MAX_TOKENS` | `64000` ceiling, clamped to the usable request budget; not sent by `openai-codex` |
| `--max-steps` | `JECODE_MAX_STEPS` | `40` |
| `--compaction-percent` | `JECODE_COMPACTION_PERCENT` | `85`; accepts `50` through `95` |
| `--reduced-motion` | `JECODE_REDUCED_MOTION=1` | Off |
| `--auto-approve` | `JECODE_AUTO_APPROVE=1` | Off |
| `--ephemeral` | `JECODE_EPHEMERAL=1` | Off |

Non-secret preferences live in `~/.jecode/settings.json`. Explicitly saved API
keys live in `~/.jecode/credentials.json`, while ChatGPT OAuth accounts live in
`~/.jecode/accounts.json`. Environment credentials always take precedence.
Secret stores use owner-only permissions where the operating system supports
them.

Jecode has one current interface theme, Dark Steel. `NO_COLOR` is supported for
terminals and pipelines that disable colour.

## Safety model

Jecode treats model output, workspace content, tool output, and terminal text as
untrusted data.

- Current filesystem tools are confined to the selected workspace. Writes
  reject symlink and junction components, revalidate boundaries and the
  approved file state immediately before atomic replacement.
- Dangerous tools ask by default unless explicitly allowed for the session or
  the process starts with `--auto-approve`.
- Credential fields are masked and excluded from transcripts. Recognized
  credential values are redacted before output reaches the model, screen,
  history, or export.
- Approved shell commands receive no secret-bearing environment variables.
  `SSH_AUTH_SOCK` is preserved so Git and SSH can use the user's agent, which
  means an approved command may ask that agent to authenticate.
- ChatGPT OAuth uses PKCE and an exact loopback callback or the OpenAI device
  flow. Refresh-token rotation is serialized across Jecode processes.
- Terminal control characters are neutralized before rendering.
- Remote Ollama endpoints require HTTPS, and provider redirects are rejected.
- Provider handshakes and idle response bodies have finite deadlines. Only
  idempotent catalogue reads retry; generation requests are never replayed.
- Model, terminal, and filesystem input are bounded before use.
- Session files are versioned, symmetrically size-bounded before write and
  after read, atomically checkpointed, and treated as untrusted when loaded. A
  live lease prevents concurrent resume.

`run_command` is not an operating-system sandbox. An approved command can still
access files and account resources available to the current user. Review
commands carefully and reserve `--auto-approve` for controlled environments.

Read [SECURITY.md](SECURITY.md) before reporting a vulnerability.

## Project direction

The terminal is Jecode's current primary interface, not the limit of the
product. Future interfaces may reuse the same controller, session model, and
visible control system. Expansion must remain deliberate: no hidden model
hierarchy, no delegated authority, and no weakening of the zero-dependency
runtime.

The single-controller rule limits delegation, not duration or scope. Jecode may
eventually supervise visible, interruptible processes such as development
servers, file watchers, and test runners. Those processes remain tools owned by
the controller; they do not receive independent goals, model loops, or tool
authority.

The selected workspace is the current default filesystem boundary. Any future
access to additional directories or resources must use explicit, reviewable,
revocable grants rather than silently widening that boundary.

## Build from source

```console
git clone https://github.com/giovannijecha/jecode.git
cd jecode
npm ci --ignore-scripts
npm run build:release
npm link
jecode --version
```

Development runs TypeScript directly with `npm run start`. `dist/` is an
ignored generated tree used only by linked commands and release tarballs.
`npm run pack:release` rebuilds it from a clean target. Installing the published
package runs no compilation or installation scripts.

Run the complete project checks with:

```console
npm run check
```

Use `npm run tui:lab` to inspect production TUI components with inert local
fixtures. `npm run bench:transcript` and `npm run bench:search` provide manual
probes for long-session rendering and workspace search. Architecture and
security boundaries are documented in
[docs/architecture.md](docs/architecture.md); brand assets and usage rules live
in [docs/brand.md](docs/brand.md). The public 1.0 contract lives in
[docs/compatibility.md](docs/compatibility.md), and the maintainer release
procedure lives in [docs/releasing.md](docs/releasing.md).

## Community

- Ask questions, share workflows, and explore early ideas in
  [GitHub Discussions](https://github.com/giovannijecha/jecode/discussions).
- Report reproducible bugs and focused feature requests through
  [GitHub Issues](https://github.com/giovannijecha/jecode/issues). New
  capabilities are deferred until the 1.0 stabilization cycle is complete.
- Report security concerns privately through the repository Security tab.

Public pull requests are not accepted at this stage. Code changes remain a
maintainer and invited-collaborator workflow. See
[CONTRIBUTING.md](CONTRIBUTING.md) and
[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## License

Jecode is available under the [MIT License](LICENSE).
