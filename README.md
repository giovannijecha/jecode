<p align="center">
  <img src="docs/assets/brand/jeco-256.png" width="128" alt="Jeco, the steel-blue Jecode gecko">
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/brand/wordmark-light.svg">
    <source media="(prefers-color-scheme: light)" srcset="docs/assets/brand/wordmark-dark.svg">
    <img src="docs/assets/brand/wordmark-dark.svg" width="280" alt="Jecode">
  </picture>
</p>

<p align="center"><strong>Your code. Your loop.</strong></p>

<p align="center">
  A focused coding agent that lives in your terminal, keeps tool use visible,
  and stays under your control.
</p>

<p align="center">
  <a href="https://github.com/giovannijecha/jecode/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/giovannijecha/jecode/actions/workflows/ci.yml/badge.svg"></a>
  <a href="LICENSE"><img alt="MIT License" src="https://img.shields.io/badge/license-MIT-669BD2"></a>
  <img alt="Node.js 24+" src="https://img.shields.io/badge/node-%3E%3D24-86CB92">
  <img alt="Runtime dependencies: zero" src="https://img.shields.io/badge/runtime_dependencies-0-8DB4DD">
</p>

<p align="center">
  <a href="https://github.com/giovannijecha/jecode/blob/main/CHANGELOG.md">Changelog</a> ·
  <a href="https://github.com/giovannijecha/jecode/releases">Releases</a>
</p>

> Jecode is an early 0.1.x release. The core loop is usable today; commands and
> terminal interactions may still evolve before 1.0.

## Why Jecode

- **One controller.** One visible loop talks to the model, runs tools, and returns
  control to you. There are no hidden workers or delegated agents.
- **Terminal-native.** The transcript, composer, searchable menus, tool output,
  diffs, approvals, reasoning, and status all share one full-screen TUI.
- **Permission-aware.** Reads stay transparent; dangerous actions ask first.
  Session approvals can be reviewed and revoked.
- **Provider-neutral.** Use Anthropic, OpenAI, or a local/remote Ollama server
  without changing the workflow.
- **Lean by construction.** Jecode installs as plain JavaScript, runs on
  Node.js 24, executes no installation scripts, and has zero third-party
  runtime dependencies.

## Install

Jecode requires **Node.js 24 or newer** and npm:

~~~console
npm install --global @giovannijecha/jecode
jecode --version
~~~

To try prereleases instead, install the opt-in **next** channel with
`npm install --global @giovannijecha/jecode@next`.

Published npm packages are the supported installation artifacts. Git URL
installs are intentionally unsupported: the source tree contains no generated
runtime and defines no install-time build hook.

Then open the project you want to work on and run Jecode:

~~~console
cd path/to/your/project
jecode
~~~

You can point at another workspace explicitly:

~~~console
jecode --root path/to/your/project
~~~

Run **jecode --help** for all startup options. Tested platforms are Windows,
Ubuntu, and macOS.

### Update

Install the current stable release over the existing global command:

~~~console
npm install --global @giovannijecha/jecode
jecode --version
~~~

### Uninstall

~~~console
npm uninstall --global @giovannijecha/jecode
~~~

Uninstalling the command preserves **~/.jecode** so saved settings and
credentials remain available after a reinstall. Remove that directory only
when you intentionally want to erase Jecode's local data.

### Linux and WSL

WSL has its own Node.js installation and `PATH`; the Node.js version installed
on Windows does not apply inside it. If installation succeeds but `jecode` is
not found, run **npm config get prefix** and ensure its **bin/** directory is on
your Linux `PATH`. Do not ignore an `EBADENGINE` warning: **node --version** must
report 24 or newer.

### Build from source

~~~console
git clone https://github.com/giovannijecha/jecode.git
cd jecode
npm ci --ignore-scripts
npm run build:release
npm link
jecode
~~~

Development runs TypeScript directly with **npm run start**. **dist/** is an
ignored, generated tree used only by linked commands and release tarballs.
**npm run pack:release** rebuilds it from a clean target before packing; the
trusted publish workflow performs the same explicit build. Installing the
published package runs no compilation or installation scripts.

## First session

Jecode opens on an empty composer instead of forcing a setup wizard. Type
**/settings** when you are ready to choose a provider, select a model, and add a
credential. A credential can remain in memory for the current session or be
saved explicitly under **~/.jecode**; it is never stored in the workspace.

| Provider | Credential | Notes |
|---|---|---|
| Anthropic | ANTHROPIC_API_KEY | Cloud |
| OpenAI | OPENAI_API_KEY | Cloud |
| Ollama | OLLAMA_API_KEY for Cloud/remote | Cloud with a key, local without one |

Choose **cloud**, **local**, or a custom endpoint from the Ollama connection row
in **/settings**. Existing users with an Ollama API key automatically use
**https://ollama.com**; without a key, Jecode uses the local daemon at
**http://127.0.0.1:11434**. Remote custom endpoints must use HTTPS.

## Use the TUI

Type **/** to open searchable command completion inside the composer.

| Command | What it does |
|---|---|
| /settings | Manage provider, connection, model, limits, motion, and credentials |
| /effort | Change and save reasoning effort directly |
| /providers | Switch the provider for the next turn |
| /models | Search the live model catalogue |
| /credentials | Add, replace, inspect, or forget saved credentials |
| /permissions | Review or revoke remembered session approvals |
| /usage | Show normalized token usage |
| /new | Start a clean in-memory conversation |
| /export | Save a timestamped Markdown transcript in the launch directory |
| /help | Show commands and controls |
| /exit | Restore the terminal and exit |

Useful controls:

- **Up/Down** moves through command suggestions, menus, and input history.
- **Tab** completes a slash command without running it; **Enter** sends.
- **Alt+Enter** inserts a newline.
- **Esc** closes a menu or interrupts the foreground operation.
- **Ctrl+C** interrupts, or exits while idle. **Ctrl+D** requests a clean exit.
- **PageUp/PageDown** and the mouse wheel scroll the transcript without losing
  the place you are reading.
- **Ctrl+O** expands or compacts the latest reasoning or tool-detail block.

The one-line footer keeps model, effort, and workspace on the left. Live work
stays visible on the reasoning and tool rail; the right edge carries the
interrupt hint, readiness guidance, and temporary feedback without polluting
the transcript.

## Configuration

Startup precedence is: command-line flags, environment variables, saved
settings, built-in defaults.

| Flag | Environment | Default |
|---|---|---|
| --provider | JECODE_PROVIDER | anthropic |
| --model | JECODE_MODEL | Provider default or interactive selection |
| --ollama-host | OLLAMA_HOST | Cloud with an Ollama key, local without one |
| --root | — | Current directory |
| --effort | JECODE_EFFORT | high |
| --max-tokens | JECODE_MAX_TOKENS | 64000 |
| --max-steps | JECODE_MAX_STEPS | 40 |
| --reduced-motion | JECODE_REDUCED_MOTION=1 | Off |
| --auto-approve | JECODE_AUTO_APPROVE=1 | Off |

Persistent preferences live in **~/.jecode/settings.json**. Explicitly saved
credentials live in **~/.jecode/credentials.json** with owner-only permissions
where the operating system supports them. Environment credentials always win.

Jecode has one interface theme: dark Steel. **NO_COLOR** is supported for
terminals and pipelines that disable colour.

## Automation

When stdin or stdout is piped, Jecode switches to a plain line-oriented mode:

~~~console
printf "explain this project\n" | jecode --root .
~~~

Dangerous tools stay denied in batch mode unless **--auto-approve** is supplied
explicitly. A terminal batch failure is written to stderr and exits non-zero,
so shell pipelines can stop reliably.

## Safety model

Jecode treats model output, workspace content, tool output, and terminal text as
untrusted data.

- Tool paths are confined to the selected workspace. Writes reject symlink and
  junction components, then revalidate the boundary during atomic replacement.
- Dangerous tools require approval unless the process was started with
  **--auto-approve**.
- Credential fields are masked and excluded from transcripts. Approved shell
  commands receive no credential-like environment variables, and recognized
  credential values are redacted before tool output reaches the model, screen,
  history, or export.
- Terminal control characters are neutralized before rendering.
- Remote Ollama endpoints require HTTPS. Provider HTTP redirects are rejected
  rather than followed across an implicit trust boundary.
- Provider handshakes and idle response bodies have finite deadlines. Only
  idempotent catalogue reads retry; generation requests are never replayed.
- Model and filesystem input are bounded before they reach the screen or
  provider.

`run_command` is not an operating-system sandbox: an approved shell command can
still access files and account resources available to the current user. Review
commands carefully and reserve **--auto-approve** for controlled environments.

Please read [SECURITY.md](SECURITY.md) before reporting a vulnerability.

## Community

Jecode is built around people using the product and telling us where the loop
can improve.

- Ask questions, share workflows, and explore ideas in
  [GitHub Discussions](https://github.com/giovannijecha/jecode/discussions).
- Report reproducible bugs and focused feature requests through
  [GitHub Issues](https://github.com/giovannijecha/jecode/issues).
- Report security concerns privately through the repository Security tab.

Public pull requests are not accepted at this stage; code changes remain a
maintainer/collaborator workflow. See [CONTRIBUTING.md](CONTRIBUTING.md) for the
short routing guide and [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) for community
expectations.

## Development

~~~console
npm ci --ignore-scripts
npm run check
~~~

The visual lab exercises the complete production TUI—golden conversation,
live tool trace, output tails, change-centric diffs, approvals, menus, and
fields—without a provider, network access, tool execution, or workspace writes:

~~~console
npm run tui:lab
~~~

For a manual long-session rendering probe, run **npm run bench:transcript**.

Architecture and security boundaries are documented in
[docs/architecture.md](docs/architecture.md). Brand assets and usage rules live
in [docs/brand.md](docs/brand.md).

## License

Jecode is available under the [MIT License](LICENSE).
