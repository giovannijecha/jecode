<p align="center">
  <img src="docs/assets/brand/jeco-256.png" width="128" alt="Jeco, the steel-blue Jecode gecko">
</p>

<p align="center">
  <img src="docs/assets/brand/wordmark-steel.svg" width="280" alt="Jecode">
</p>

<p align="center"><strong>Your code. Your loop.</strong></p>

<p align="center">
  A focused coding agent that lives in your terminal, keeps tool use visible,
  and stays under your control.
</p>

<p align="center">
  <a href="https://github.com/giovannijecha/jecode/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/giovannijecha/jecode/actions/workflows/ci.yml/badge.svg"></a>
  <a href="LICENSE"><img alt="MIT License" src="https://img.shields.io/badge/license-MIT-669BD2"></a>
  <img alt="Node.js 22.18+ or 24+" src="https://img.shields.io/badge/node-22.18%2B%20%7C%2024%2B-86CB92">
  <img alt="Runtime dependencies: zero" src="https://img.shields.io/badge/runtime_dependencies-0-8DB4DD">
</p>

<p align="center">
  <a href="https://github.com/giovannijecha/jecode/blob/main/CHANGELOG.md">Changelog</a> ·
  <a href="https://github.com/giovannijecha/jecode/releases">Releases</a>
</p>

> Jecode is an early 0.3.x release. The core loop is usable today; commands and
> terminal interactions may still evolve before 1.0.

## Why Jecode

- **One controller.** One visible loop talks to the model, runs tools, and returns
  control to you. Independent reads can overlap inside one step; writes and
  commands remain ordered. There are no hidden workers or delegated agents.
- **Terminal-native.** The transcript, composer, searchable menus, tool output,
  diffs, approvals, reasoning, and status all share one full-screen TUI.
- **Permission-aware.** Reads stay transparent; dangerous actions ask first.
  Session approvals can be reviewed and revoked.
- **Provider-neutral.** Use Anthropic or OpenAI API keys, a ChatGPT account, or
  a local/remote Ollama server without changing the workflow.
- **Lean by construction.** Jecode installs as plain JavaScript, runs on
  Node.js 22.18+ (22.x) or Node.js 24+, executes no installation scripts, and
  has zero third-party runtime dependencies.

## Install

Jecode requires **Node.js 22.18+ on the 22.x line, or Node.js 24+**, and npm:

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

If you installed Jecode from GitHub before the scoped npm package existed,
remove the old unscoped package once before updating:

~~~console
npm uninstall --global jecode
npm install --global @giovannijecha/jecode
~~~

An `EEXIST` error for a `bin/jecode` path usually means that this legacy
executable still owns the command. Remove it instead of installing with
`--force`.

### Uninstall

~~~console
npm uninstall --global @giovannijecha/jecode
~~~

Uninstalling the command preserves **~/.jecode** so saved settings and
credentials remain available after a reinstall. Remove that directory only
when you intentionally want to erase Jecode's local data.

### Linux and WSL

WSL has its own Node.js installation and `PATH`; the Node.js version installed
on Windows does not apply inside it. Keep user-installed npm commands in the
Linux user path and put that path before inherited Windows entries:

~~~console
npm config set prefix "$HOME/.local"
export PATH="$HOME/.local/bin:$PATH"
npm install --global @giovannijecha/jecode
hash -r
command -v jecode
jecode --version
~~~

Persist the `PATH` export in `~/.profile` or your shell's startup file. Inside
WSL, **command -v jecode** should resolve below `/home/...`, not below
`/mnt/c/.../Volta`. Do not ignore an `EBADENGINE` warning: **node --version**
must report 22.18+ on the 22.x line, or 24+.

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
**/settings** when you are ready to choose a provider, select a model, and
configure authentication. An API key can remain in memory for the current
session or be saved explicitly under **~/.jecode**; it is never stored in the
workspace.

| Provider ID | Authentication | Notes |
|---|---|---|
| anthropic | ANTHROPIC_API_KEY | Anthropic API |
| openai | OPENAI_API_KEY | OpenAI API |
| openai-codex | ChatGPT OAuth | Experimental; uses eligible ChatGPT Codex access |
| ollama | OLLAMA_API_KEY for Cloud/remote | Cloud with a key, local without one |

Choose **openai-codex** to sign in on OpenAI's website without pasting a key.
Jecode offers a local browser callback and a device-code flow; WSL and remote
terminals default to the device code. The connection is saved only after the
flow completes. Availability and usage limits are determined by the ChatGPT
account and plan, not by OpenAI API credits. This integration is experimental
and is not an endorsement of Jecode by OpenAI.

Anthropic remains API-key only. Jecode does not reuse a Claude consumer
subscription or copy credentials from another client.

Choose **cloud**, **local**, or a custom endpoint from the Ollama connection row
in **/settings**. Existing users with an Ollama API key automatically use
**https://ollama.com**; without a key, Jecode uses the local daemon at
**http://127.0.0.1:11434**. Remote custom endpoints must use HTTPS.

## Use the TUI

Type **/** to open searchable command completion inside the composer.

| Command | What it does |
|---|---|
| /settings | Manage provider, connection, model, limits, motion, and authentication |
| /effort | Change and save reasoning effort directly |
| /providers | Switch the provider for the next turn |
| /models | Search the live model catalogue |
| /credentials | Manage API keys and the connected ChatGPT account |
| /permissions | Manage session tool access and remembered approvals |
| /new | Start a clean conversation and reset session tool permissions |
| /export | Save a timestamped Markdown transcript in the launch directory |
| /help | Open a temporary keyboard reference in the composer dock |
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
the transcript. Slash commands never append content to the conversation or its
Markdown export; **/help** closes with **Esc**, and token accounting remains
internal to the active session.

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
| --max-tokens | JECODE_MAX_TOKENS | 64000; not sent by openai-codex |
| --max-steps | JECODE_MAX_STEPS | 40 |
| --reduced-motion | JECODE_REDUCED_MOTION=1 | Off |
| --auto-approve | JECODE_AUTO_APPROVE=1 | Off |

Persistent preferences live in **~/.jecode/settings.json**. Explicitly saved
API keys live in **~/.jecode/credentials.json**; the ChatGPT OAuth account lives
separately in **~/.jecode/accounts.json**. Both secret stores use owner-only
permissions where the operating system supports them. Environment API keys
always win.

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
- Dangerous tools ask by default unless explicitly allowed for the session in
  **/permissions** or the process started with **--auto-approve**.
- Credential fields are masked and excluded from transcripts. Approved shell
  commands receive no secret-bearing environment variables; `SSH_AUTH_SOCK`
  is preserved so Git and SSH can use the user's agent, which means an approved
  command can request that agent to authenticate. Recognized credential values
  are redacted before tool output reaches the model, screen, history, or export.
- ChatGPT OAuth uses PKCE and an exact loopback callback or the OpenAI device
  flow. Refresh-token rotation is serialized across Jecode processes; OAuth
  tokens are withheld and redacted like API keys.
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
