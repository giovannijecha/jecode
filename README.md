# Jecode

[![CI](https://github.com/giovannijecha/jecode/actions/workflows/CI.yml/badge.svg)](https://github.com/giovannijecha/jecode/actions/workflows/CI.yml)

A terminal coding agent written in Rust. Jecode currently connects to OpenRouter and works
in your project through eight tools: `read`, `write`, `edit`, `bash`, `process`,
`checkpoint`, `web` and `browser`.
It has a fullscreen TUI, saved conversations and a CLI for single prompts and pipes.

One Cargo package, the Rust standard library, no external crates. Installed system
tools provide HTTPS, Bash execution and native terminal support.
`web` uses local curl for HTTP retrieval and crawling. `browser` controls an
already installed Chromium-based browser through its local DevTools interface;
set `JECODE_BROWSER` to its executable path if auto-discovery does not find it.

## Install

Build from this repository with Rust 1.95.0 and a native linker.

| Platform | System requirements |
| --- | --- |
| Windows 10+ | Git Bash, HTTPS-enabled curl, Windows PowerShell with `Add-Type`. |
| Linux / macOS | `bash`, HTTPS-enabled `curl`, `kill`, `mkfifo`, `rm`, `rmdir`; `stty`, `/dev/tty` and a VT-compatible terminal for the TUI. |

The TUI uses 24-bit color. On macOS, install Apple's Command Line Tools for the
native linker. See [platform verification](docs/development/PLATFORMS.md) for
executed coverage and checks that still need a native host.

### Windows

From the repository root in PowerShell:

```powershell
.\INSTALL.ps1
```

This builds offline, installs to `%LOCALAPPDATA%\Jecode\bin` and adds that directory
first in your user PATH. The current terminal is ready immediately; reopen other
terminals to refresh their PATH. Run the script again after source updates.

The installer prefers the installed `1.95.0-x86_64-pc-windows-gnullvm` toolchain
and its bundled linker; it needs Rust's `rust-mingw` component. Other toolchains
use their configured native linker; MSVC needs the Visual C++ Build Tools.

### Linux / macOS

From the repository root in a shell:

```sh
sh INSTALL.sh
export PATH="${XDG_DATA_HOME:-$HOME/.local/share}/jecode/bin:$PATH"
```

Add the PATH line to your shell configuration for future terminals. Run the same
script to update. An absolute install root is also accepted:
`sh INSTALL.sh /absolute/path`.

## First launch

Open the project directory you want Jecode to work in, then run:

```text
jecode
```

1. Paste your [OpenRouter API key](https://openrouter.ai/settings/keys); input is hidden.
2. Choose a tool-capable model from the searchable list, or enter `=provider/model-id`.
3. Type a task and press Enter.

Setup validates the key without sending a paid chat completion. `/cancel` leaves
setup without replacing existing settings. The key and defaults are stored in
plaintext at `~/.jecode/config.json`.

## Work in a project

The draft stays editable during a turn. Enter sends a follow-up before the next
model request; Ctrl+J inserts a newline. Ctrl+Q quits and saves input in an existing
saved session. See [composer and controls](docs/tui/COMPOSER.md) for keyboard shortcuts.

| Command | Purpose |
| --- | --- |
| `/new` | Start a fresh conversation with saved defaults. |
| `/resume [ID]` | Choose a saved conversation from this project. |
| `/model [ID]`, `/effort [NAME]` | Change this conversation's model or reasoning effort. |
| `/backend [ID [MODEL]]` | List available backends or switch this conversation to a configured backend and model. |
| `/routing` | Choose Auto routing or a specific OpenRouter endpoint. |
| `/settings` | Edit saved defaults and the OpenRouter key. |
| `/init` | Inspect this project and create or update its `JECODE.md`. |
| `/skills` | List personal skills available to the model. |
| `/checkpoints` | Inspect or recover files from a tool call. |
| `/help` | Show commands and controls. |
| `/exit` | Quit when the active turn finishes. |

Use [`JECODE.md`](docs/usage/PROJECT_INSTRUCTIONS.md) for project rules, or ask
`/init` to help write it. Jecode reads instruction files in the launch directory
and its subdirectories before each new turn. Each file applies to its own
directory and descendants; instructions above the launch directory are excluded.

Keep reusable workflows in [`~/.jecode/skills`](docs/usage/SKILLS.md). Jecode
shows their names and descriptions to the model, which reads a skill when it
fits the task. `/skills` lists the library without making a model request.

Jecode saves local preimages of modifying tool calls in
[checkpoints](docs/usage/CHECKPOINTS.md). It saves a file's previous state
before `write` and `edit` change it. Bash saves declared files and recursive
directories through `protect`; use `"."` to opt into the whole eligible
workspace. Startup and turns do not scan the project. You or the model can
inspect and restore saved states; restores can be undone. Jecode manages packs
under `~/.jecode/checkpoints/`, and each pack expires after 24 hours.

Conversations autosave under `~/.jecode/sessions/`. To resume, run
`jecode resume` from the original project directory. See [saved sessions](docs/usage/SESSIONS.md)
for unsent input, interruption and deletion.

Jecode runs tools with your system permissions. Its working directory is not a
sandbox. Provider requests send conversation content and supported attachments to
OpenRouter and the selected provider; see [security](SECURITY.md).

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
