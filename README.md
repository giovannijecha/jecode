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

> Jecode is an early 0.7.x release. The core loop is usable today; commands and
> terminal interactions may still evolve before 1.0.

## Why Jecode

- **One controller.** One visible loop talks to the model, runs tools, and returns
  control to you. Independent reads can overlap inside one step; writes and
  commands remain ordered. There are no hidden workers or delegated agents.
- **Terminal-native.** The transcript, composer, searchable menus, tool output,
  diffs, approvals, reasoning, and status all share one full-screen TUI.
- **Permission-aware.** Reads stay transparent; dangerous actions ask first.
  Session approvals can be reviewed and revoked.
- **Durable by default.** Interactive conversations survive terminal exits and
  can be resumed without replaying tools. Batch runs remain stateless.
- **Context-bounded.** Older model context is summarized automatically while
  the complete conversation and transcript remain available in the session.
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

Resume a saved conversation for the current project with a searchable picker,
or open the most recent one directly:

~~~console
jecode resume
jecode resume --latest
~~~

Use `jecode --ephemeral` when a conversation must stay memory-only.

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
**/providers** to connect the services you use, then **/models** to choose from
their combined live catalogues. **/settings** keeps the selected model and the
remaining non-secret defaults together. An API key can remain in memory for
the current session or be saved explicitly under **~/.jecode**; it is never
stored in the workspace.

| Provider ID | Authentication | Notes |
|---|---|---|
| anthropic | ANTHROPIC_API_KEY | Anthropic API |
| openai | OPENAI_API_KEY | OpenAI API |
| openai-codex | ChatGPT OAuth | Experimental; uses eligible ChatGPT Codex access |
| ollama | OLLAMA_API_KEY for Cloud/remote | Cloud with a key, local without one |

Choose **ChatGPT** in **/providers** to sign in on OpenAI's website without
pasting a key.
Jecode offers a local browser callback and a device-code flow; WSL and remote
terminals default to the device code. The connection is saved only after the
flow completes. Availability and usage limits are determined by the ChatGPT
account and plan, not by OpenAI API credits. This integration is experimental
and is not an endorsement of Jecode by OpenAI.

Anthropic remains API-key only. Jecode does not reuse a Claude consumer
subscription or copy credentials from another client.

Choose **Ollama** in **/providers** to manage its API key and select **cloud**,
**local**, or a custom endpoint. Existing users with an Ollama API key
automatically use **https://ollama.com**; without a key, Jecode uses the local
daemon at **http://127.0.0.1:11434**. Remote custom endpoints must use HTTPS.

## Use the TUI

Type **/** to open searchable command completion inside the composer.

| Command | What it does |
|---|---|
| /settings | Manage the selected model, limits, context compaction, effort, motion, and provider access |
| /effort | Change and save reasoning effort directly |
| /providers | Manage API keys, ChatGPT sign-in, and Ollama connections |
| /models | Search models across every available provider and select one |
| /permissions | Change session tool access inline and review remembered approvals |
| /new | Close the current conversation, start clean, and reset tool permissions |
| /timeline | Navigate completed turns and select where the next branch starts |
| /compact | Compact the active model context immediately |
| /export | Save a timestamped Markdown transcript in the launch directory |
| /help | Open a temporary keyboard reference in the composer dock |
| /exit | Restore the terminal and exit |

Useful controls:

- **Up/Down** moves through command suggestions, menus, and input history.
- **Left/Right** moves the composer cursor or changes an inline menu value;
  **Ctrl+Left/Right** moves by word.
- **Backspace/Delete** removes one character; **Ctrl+Backspace/Delete** removes
  one word. **Home/End** moves to the start or end of the composer.
- **Tab** completes a slash command without running it; **Enter** sends.
- **Alt+Enter** inserts a newline.
- **Esc** closes a menu or interrupts the foreground operation.
- **Ctrl+C** interrupts, or exits while idle. **Ctrl+D** requests a clean exit.
- **PageUp/PageDown** and the mouse wheel scroll the transcript without losing
  the place you are reading.
- **Ctrl+O** expands or compacts the latest reasoning or tool-detail block.

The one-line footer keeps model, effort, and workspace on the left. While work
is active, the right edge shows its current state, elapsed time, and interrupt
hint; readiness guidance and temporary feedback use the same replaceable space
without polluting the transcript. Slash commands never append content to the
conversation or its Markdown export; **/help** closes with **Esc**, and token
accounting remains internal to the active session.

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
| --compaction-percent | JECODE_COMPACTION_PERCENT | 85; accepts 50 through 95 |
| --reduced-motion | JECODE_REDUCED_MOTION=1 | Off |
| --auto-approve | JECODE_AUTO_APPROVE=1 | Off |
| --ephemeral | JECODE_EPHEMERAL=1 | Off |

Persistent preferences live in **~/.jecode/settings.json**. Explicitly saved
API keys live in **~/.jecode/credentials.json**; the ChatGPT OAuth account lives
separately in **~/.jecode/accounts.json**. Both secret stores use owner-only
permissions where the operating system supports them. Environment API keys
always win. Model selection saves the provider and model as one change; the
separate startup flags remain available for automation and override that saved
choice.

Interactive conversations are stored under **~/.jecode/sessions**, scoped to
the canonical workspace path. A checkpoint contains normalized messages and
the settled transcript needed to redraw the conversation. It excludes stored
provider credentials, OAuth tokens, provider-only opaque response data,
permission choices, draft composer text, transient footer notices, and pending
tool state. Session
files use owner-only modes on POSIX; Windows relies on the user-profile ACL.
`jecode resume` keeps the same durable session identity and advances that
session's conversation tree, so reopening and continuing a conversation does
not create duplicate picker entries. `/new` or a fresh launch starts another
logical session. **/timeline** shows the completed turns in that tree. Selecting
an earlier turn changes only the visible path; it creates and persists a branch
only when the next real message is sent. Cancelling the picker or exiting first
leaves the durable head untouched, and resume returns to the last branch with a
persisted turn. Historical tools are displayed but never executed. If a crash
left the newest turn inside a tool loop, the same session resumes from its
latest completed ancestor and the next turn becomes a branch because
provider-only continuation data is intentionally not stored. **/export** writes
only the currently selected path.

When the model-facing context approaches the selected model's usable capacity,
Jecode asks the provider for one bounded summary of its older prefix and keeps
the recent turn exact. The trigger defaults to 85% and can be changed from 50%
through 95% in **/settings**. Live provider metadata or Ollama's allocated
runtime context determines the budget when available; a metadata failure falls
back safely without blocking the turn. Only the provider projection is
replaced: complete messages, tool evidence, transcript, and conversation tree
remain unchanged. The branch-local summary anchor is checkpointed with the
session, so resume reuses it instead of summarizing the same prefix again. A
failed or cancelled optional summary leaves the original context intact; a
definite provider context-limit rejection may trigger one compacted retry.
Internal summary requests count toward provider usage but never appear in the
transcript or Markdown export. **/compact** requests the same model-aware,
branch-local compaction immediately, even below the automatic trigger. Very
small contexts are left unchanged. After selecting a historical branch point,
send its first new message before compacting so shared history is never
rewritten.

Jecode has one interface theme: dark Steel. **NO_COLOR** is supported for
terminals and pipelines that disable colour.

## Automation

When stdin or stdout is piped, Jecode switches to a plain line-oriented mode:

~~~console
printf "explain this project\n" | jecode --root .
~~~

Batch conversations are never written to the session store.

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
- Durable session files are versioned, size-bounded, atomically checkpointed,
  and treated as untrusted when loaded. A live lease prevents the same saved
  session from being resumed by two Jecode processes at once.

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
