# Jecode

[![CI](https://github.com/giovannijecha/jecode/actions/workflows/CI.yml/badge.svg)](https://github.com/giovannijecha/jecode/actions/workflows/CI.yml)

[Contributing](CONTRIBUTING.md) · [Security](SECURITY.md)

A personal terminal coding agent written in Rust, using OpenRouter and four tools:
`read`, `write`, `edit` and `bash`. Tools execute directly in your project.

Jecode uses one Cargo package and the Rust standard library. There are no
external crates or third-party runtime libraries. Installed system
executables provide HTTPS, Bash, process supervision and terminal input.

## Start on Windows

You need Windows 10 or newer, Rust 1.95.0 with its native linker, Git Bash and
HTTPS-enabled curl.
Jecode discovers Git Bash automatically and uses Windows' installed curl.
Windows execution uses installed Windows PowerShell and .NET's built-in
`Add-Type` compiler for an owned process supervisor, compiled once per source
version and reused from the user-scoped cache. The interactive interface
also uses Windows console APIs.

From this repository in PowerShell, install once:

```powershell
.\INSTALL.ps1
```

The script builds offline, installs the Rust executable in
`%LOCALAPPDATA%\Jecode\bin` and puts that directory first in your personal PATH
and the current terminal's PATH. It preserves other installations, including any
older `jecode` command from Volta. Reopen other terminals to refresh their PATH.
Run the same script after changing Jecode's source to update the installed binary.

Open the directory of the project you want to work on, then run:

```powershell
jecode
jecode --plain
```

## Start on Linux and macOS

You need Rust 1.95.0 with a native linker, plus HTTPS-enabled `curl`, `bash`,
`kill`, `mkfifo`, `rm` and `rmdir` on PATH. The TUI also needs `stty`, `/dev/tty`
and a VT-compatible terminal.
On macOS, the native linker is supplied by Apple's Command Line
Tools. From this repository in a shell:

```sh
sh INSTALL.sh
export PATH="${XDG_DATA_HOME:-$HOME/.local/share}/jecode/bin:$PATH"
jecode
```

The script builds offline and installs under
`${XDG_DATA_HOME:-$HOME/.local/share}/jecode`. Add the PATH line to your shell's
configuration to use the command in future terminals. Run the same script to
update this installation. An optional absolute install root supports isolated
installations: `sh INSTALL.sh /absolute/path`.

Windows, Linux and macOS share the same multiline TUI, including the editor,
message queue, selectors, streaming and saved sessions. Single prompts, pipes
and `--plain` retain the line-based CLI. Linux installation and TUI execution
have been tested in WSL; macOS has been compile-checked for Intel and Apple Silicon,
with native terminal verification still required.

## First launch

The first launch guides you through three steps:

1. Paste your [OpenRouter API key](https://openrouter.ai/settings/keys). Input is hidden.
2. Search for a model by name and choose a number. Only tool-capable models are
   listed, with input/output prices per million tokens when available. No model is
   selected automatically. You can also enter `=provider/model-id` directly.
3. Jecode saves your settings and opens a conversation. Type a task and press Enter.

The key is checked with OpenRouter before saving. Setup does not send a paid chat
completion. Type `/cancel` to leave setup without replacing your settings.

## Daily use

```text
jecode
jecode "Explain the structure of this project"
jecode --model provider/model-id "Check this change"
jecode --attach report.pdf "Summarize this report"
jecode setup
```

| Command in chat | Action |
| --- | --- |
| `/new` | Start a new conversation using the saved defaults. |
| `/resume [ID]` | Resume a conversation; Ctrl+D in the list marks it for deletion, Enter confirms. |
| `/drafts` | Review pending messages; edit one with Enter, discard with Ctrl+D, or send a paused draft with Ctrl+S. |
| `/attach PATH...` | Attach files to the draft; dropping files or Alt+V for a clipboard image does the same. |
| `/model [ID]` | Choose the current conversation's model, then its effort. |
| `/effort [NAME]` | Choose a supported reasoning effort for this conversation. |
| `/settings` | Change the saved model, effort or OpenRouter key. |
| `/help` | Show command and keyboard help. |
| `/copy` | Choose the last completed response, a code block or a quote to copy. |
| `/export` | Save the current conversation as JSON in the working directory. |
| `/tmp [clean]` | Show session temporary files, or explicitly clear that area. |
| `/exit` | Quit after cancellation and cleanup of any active work. |

`/clear` and `/setup` remain aliases for `/new` and `/settings`.
Model and effort changes keep the conversation and do not overwrite saved
defaults. Settings apply to future `/new` conversations and launches; changing
the key also updates authentication for the current conversation.

Interactive stdin and stdout open the fullscreen TUI. The draft stays editable
during work; Enter queues messages and Alt+Up opens pending drafts. Ctrl+J inserts
a newline, Ctrl+P/N browses sent prompts without commands, and **Ctrl+Q quits**,
cancelling active work and saving input in a saved session. Panels, draft edits
and history recall receive Esc first. While reading, the blue Back to bottom
badge offers Esc to follow the latest output; otherwise Esc stops work. Ctrl+C
stops work when no panel, edit or history recall has priority. Page Up/Down and
the mouse wheel scroll the conversation; Alt+Home/End jumps to its start/latest
output and Alt+T inspects tools.
See [composer and controls](docs/tui/COMPOSER.md) for the full keyboard reference.

Use `jecode --plain` for line-based chat. Single prompts and redirected input/output
use the CLI: progress goes to stderr and answers go to stdout. End-of-input exits
line-based chat. Use `--plain` if the native terminal adapter is unavailable.
Options precede the prompt; use `--` before a prompt starting with `-` or a
command word used as a task. From your project directory, a piped task looks like:

```powershell
'Explain this project' | jecode
```

Piped tasks require existing configuration and never start interactive setup.
Failed tasks exit with code 1; invalid arguments exit with code 2. `--help` and
`--version` need no configuration or native tools.

Add [`JECODE.md`](docs/usage/PROJECT_INSTRUCTIONS.md) to the directory where you
run Jecode for project instructions. It is read before each new user request;
changes apply on the next request, including after resuming a saved session.

Settings live in `~/.jecode/config.json`; the OpenRouter key is stored in plain
text. Conversations autosave from their first sent request under
`~/.jecode/sessions/`. Resume explicitly from the same project directory:

```text
jecode resume
jecode resume SESSION_ID
jecode --plain resume SESSION_ID
```

## Documentation

The [documentation index](docs/README.md) groups the detailed guides:

| Area | Start here |
| --- | --- |
| Usage | [Configuration](docs/usage/CONFIGURATION.md), [project instructions](docs/usage/PROJECT_INSTRUCTIONS.md), [tools](docs/usage/TOOLS.md) and [sessions](docs/usage/SESSIONS.md). |
| Terminal interface | [Composer and controls](docs/tui/COMPOSER.md), [Markdown](docs/tui/MARKDOWN.md) and [theme/tool cards](docs/tui/THEME.md). |
| Development | [Architecture](docs/development/ARCHITECTURE.md) and [testing](docs/development/TESTING.md). |

## Development

Run from this repository with the pinned Rust 1.95.0 toolchain:

```text
cargo build --locked --offline
cargo fmt --all -- --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline
```

`cargo run --locked --offline` starts the development binary without installation.
Tests use isolated fixtures and synthetic credentials. See
[development and testing](docs/development/TESTING.md) for platform coverage and
native/manual checks.

## License

MIT. See [LICENSE](LICENSE).
