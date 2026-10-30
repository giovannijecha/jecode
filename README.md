# Jecode

[![CI](https://github.com/giovannijecha/jecode/actions/workflows/CI.yml/badge.svg)](https://github.com/giovannijecha/jecode/actions/workflows/CI.yml)

A terminal coding agent in Rust.

Jecode connects to OpenRouter with an API key or to OpenAI through your ChatGPT
account. It works in your project through fifteen tools: `read`, `list`,
`search`, `map`, `write`, `edit`, `bash`, `exec`, `process`, `checkpoint`, `web`,
`browser`, `agent`, `ask_user` and `todo_list`. `map` surveys code structure and can save or forget sourced
notes; `list` and `search` discover files and matching lines.
The main agent can delegate focused work to collaborators with the same operational
tools and separate contexts. Collaborators never delegate. See
[collaborating agents](docs/usage/COLLABORATION.md).
The main agent can ask for an explicit answer and maintain a saved conversation
plan. See [questions and plans](docs/usage/WORK_TOOLS.md).
It has a fullscreen TUI, saved conversations and a CLI for single prompts and pipes.

One Cargo package, the Rust standard library, no external crates. Jecode owns
its command interpreter and uses operating-system APIs for HTTPS, cryptography,
processes and terminal support. Its native adapters are compiled when the
package is built and embedded in the executable: runtime never invokes a
compiler, Git, Git Bash, curl or the OpenSSL command-line application.

`exec` runs a program with separate arguments and can open a controllable
terminal session. `bash` runs [Jecode's own command language](docs/usage/SHELL.md) on every platform;
it does not select an installed shell. Isolated collaborators use owned file
snapshots and work in folders without a Git repository.

`browser` controls an installed Chromium-based browser through its local
DevTools interface. This is the application prerequisite: set `JECODE_BROWSER`
to its executable path if discovery does not find it. Programs explicitly run
for a user's project remain that project's requirements.

## Install

Use the prebuilt package for your OS and architecture. The package contains
`jecode` (`jecode.exe` on Windows), its installer and the license. It can also
run directly without installation. No Rust toolchain is required to use it.

| Platform | Native OS boundary |
| --- | --- |
| Windows | Windows console/process APIs and .NET Framework; PowerShell for system clipboard/image APIs. ConPTY is needed for interactive terminals. |
| Linux | libc/POSIX and the OS OpenSSL 3 or 1.1 ABI for TLS/cryptography. Desktop clipboard/image APIs use X11 or Wayland data control and GdkPixbuf. |
| macOS | POSIX, Foundation, Security/CommonCrypto and system clipboard/image APIs. |

Workspace command confinement additionally requires native Windows Process
Security Environment 1.1. Interactive terminals and browser control currently
require Full. Unsupported hosts reject confined command launch, and local
servers must currently start in Full. See [permissions](docs/usage/PERMISSIONS.md)
for scope, approvals and platform limits.

ChatGPT browser sign-in opens the system browser; its callback listens only on
`127.0.0.1`. HTTPS uses OS certificate verification. Jecode does not install or
download additional applications to enable its tools.

### Windows

From the extracted package in PowerShell:

```powershell
.\INSTALL.ps1
```

This copies the supplied executable to `%LOCALAPPDATA%\Jecode\bin` and adds
that directory first in the user PATH. Reopen other terminals to refresh their
PATH. Run the installer from a newer package to update. `-NoPath` keeps PATH
unchanged; `-InstallRoot` chooses an absolute destination.

### Linux / macOS

From the extracted package:

```sh
sh INSTALL.sh
export PATH="${XDG_DATA_HOME:-$HOME/.local/share}/jecode/bin:$PATH"
```

Add the PATH line to your shell configuration. The installer copies the supplied
executable; it does not build source. An absolute install root is also accepted:
`sh INSTALL.sh /absolute/path`.

### Building from source

Development needs Rust 1.95.0, a linker and the platform compiler for the owned
native adapters (.NET C# on Windows; system C/Objective-C on Unix). These are
build requirements, not application prerequisites. Build offline with
`cargo build --release --locked --offline`; `PACKAGE.ps1` or `PACKAGE.sh` prepares
a local distribution with its installer. Nothing is uploaded or published.

The TUI uses 24-bit color, ignores `NO_COLOR`, and keeps dark terminal
backgrounds. On a detected light background it paints a local dark surface;
the terminal's palette stays unchanged. See
[platform verification](docs/development/PLATFORMS.md) for executed coverage.

## First launch

Open the project directory you want Jecode to work in, then run:

```text
jecode
```

1. The TUI opens even without credentials. Open `/settings` → **Providers** to connect a ChatGPT account or add an [OpenRouter API key](https://openrouter.ai/settings/keys).
2. Choose a model with `/model`, or set the default in `/settings` → **Default model**.
3. Type a task and press Enter.

`jecode setup` also provides guided configuration and checks access without
sending an inference request. `/cancel` leaves saved defaults unchanged. Saved
defaults and the OpenRouter API key live in plaintext at `~/.jecode/config.json`;
protected ChatGPT account records live separately under
`~/.jecode/accounts/openai/`. OpenAI API keys are not supported.

First-time configuration is saved after choosing a default model. Until then,
an OpenRouter key and settings changes stay in memory for the current session;
cancelling model selection leaves the configuration file absent. Connecting a
provider and choosing its first model also activates it for the current conversation.

Use `/settings` → **Providers** → **Credentials** for ChatGPT accounts, or
**API keys** for OpenRouter. With multiple configured providers, `/model` asks
for the provider before the model. ChatGPT plan usage is managed in
[ChatGPT Settings → Usage](https://chatgpt.com/settings/usage).

If the default provider's credentials are removed, the TUI uses another configured
provider for the conversation and shows a warning. It restores that provider's
saved model when available; otherwise, use `/model` to choose one. With no
configured provider, the warning points to `/settings`. Saved defaults stay unchanged.
The warning includes the failure reason and reports when a `--model` override is
skipped because the fallback provider uses its own model identifiers.

## Work in a project

The draft stays editable during a turn. Enter sends a follow-up before the next
model request; Ctrl+J inserts a newline. Ctrl+Q quits, asking first while a turn
or background process runs, and saves input in an existing saved session. See [composer and controls](docs/tui/COMPOSER.md) for keyboard shortcuts.

The TUI groups two or more related routine calls into Work chapters with a
short result summary; single calls keep their normal cards. `Ctrl+O` opens the
complete tool cards inline for the latest finished turn; see
[tool presentation](docs/tui/THEME.md).

| Command | Purpose |
| --- | --- |
| `/new` | Start a fresh conversation with saved defaults. |
| `/resume [ID]` | Choose a saved conversation from this project. |
| `/model [ID]`, `/effort [NAME]` | Change this conversation's model or reasoning effort. |
| `/routing` | Choose Auto routing or a specific OpenRouter endpoint. |
| `/permissions [LEVEL]` | Choose Read, Workspace or Full for this conversation, and review remembered grants. |
| `/settings` | Edit saved defaults and manage provider accounts or API keys. |
| `/init` | Inspect this project and create or update its `AGENTS.md`. |
| `/skills` | List personal skills available to the model. |
| `/checkpoints` | Inspect or recover files from a tool call. |
| `/agents` | Inspect open collaborator assignments and results; review and closure belong to the main agent. |
| `/ask` | Return to a pending question in the TUI. |
| `/todo` | Inspect this conversation's current plan, without changing it or calling the model. |
| `/help` | Show commands and controls. |
| `/exit` | Quit when the active turn finishes. |

Use [`AGENTS.md`](docs/usage/PROJECT_INSTRUCTIONS.md) for project rules, or ask
`/init` to help write it. Jecode reads instruction files in the launch directory
and its subdirectories before each new turn. Each file applies to its own
directory and descendants; instructions above the launch directory are excluded.

Keep reusable workflows in [`~/.jecode/skills`](docs/usage/SKILLS.md). Jecode
shows their names and descriptions to the model, which reads a skill when it
fits the task. `/skills` lists the library without making a model request.

Jecode saves local preimages of modifying tool calls in
[checkpoints](docs/usage/CHECKPOINTS.md). It saves a file's previous state
before `write` and `edit` change it. Bash and Exec save declared files and recursive
directories through `protect`; use `"."` to opt into the whole eligible
workspace. Startup and turns do not scan the project. You or the model can
inspect and restore saved states; restores can be undone. Jecode manages packs
under `~/.jecode/checkpoints/`, and each pack expires after 24 hours.

Conversations autosave under `~/.jecode/sessions/`. To resume, run
`jecode resume` from the original project directory. See [saved sessions](docs/usage/SESSIONS.md)
for unsent input, interruption and deletion.

Each conversation has a [permission level](docs/usage/PERMISSIONS.md): Read,
Workspace (the default) or Full. In Workspace, file access outside the working
folder, or a command run from outside it, asks first. Levels govern Jecode's tool
calls, not what a command does once it runs: commands run with your system
permissions, and the working directory is not a sandbox. Provider requests send conversation content and supported attachments to
the selected backend: OpenAI directly or OpenRouter and its selected provider;
see [security](SECURITY.md).

## CLI and pipes

Run these from the project directory after setup:

```powershell
jecode "Explain the structure of this project"
jecode --model provider/model-id "Check this change"
jecode --attach report.pdf "Summarize this report"
'Explain this project' | jecode
jecode --plain
```

Replace `provider/model-id` and `report.pdf` with your model and file. Options
precede the prompt; use `--` when the prompt starts with `-` or a command word
such as `resume`. Single prompts and pipes write answers to stdout and progress
to stderr. Piped input requires existing configuration.

`--plain` opens line-based chat and also works without the native TUI adapter.
Task failures exit with code 1; invalid arguments use code 2. `--help` and
`--version` need no configuration.

`jecode checkpoints` inspects and restores the project's checkpoints without a
key, provider or network, for example `jecode checkpoints list` or
`jecode checkpoints restore rc-ID src/main.rs --expected latest`. Run `jecode checkpoints help`
for every command.

## Documentation

The [documentation index](docs/README.md) links the usage guides and technical references.

- [Configuration](docs/usage/CONFIGURATION.md): models, effort, routing and environment overrides.
- [Permissions](docs/usage/PERMISSIONS.md): levels, requests, remembered grants and their boundary.
- [Tools](docs/usage/TOOLS.md): files, commands, processes, local web retrieval and browser control.
- [Checkpoints](docs/usage/CHECKPOINTS.md): on-demand recovery, restores, scope and storage.
- [Attachments](docs/usage/ATTACHMENTS.md), [copy](docs/usage/COPY.md) and [export](docs/usage/EXPORT.md).
- [Architecture](docs/development/ARCHITECTURE.md) and [development checks](docs/development/TESTING.md).

## Development

From the repository root with the pinned toolchain and native linker. On Windows,
see [toolchain selection](docs/development/TESTING.md#windows-toolchain):

```text
cargo build --locked --offline
cargo fmt --all -- --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline
```

`cargo run --locked --offline` starts the development binary. Read
[CONTRIBUTING.md](CONTRIBUTING.md) before submitting a change.

MIT license. See [LICENSE](LICENSE).
