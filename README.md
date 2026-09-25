# Jecode

An owned coding harness for your terminal, written in Rust.

Jecode streams model responses, reads local project files, changes files and
runs commands directly. Conversations live in the terminal's natural
scrollback. Sessions, settings and account credentials belong to your user profile.

The application uses the Rust standard library and original code, with no external
crates, vendored libraries or runtime helpers for networking and presentation.
One controller owns the conversation and orders tool effects.

## Try it

Jecode is an **alpha** for Windows and Linux.

Windows x64, using npm:

```text
npm install -g @giovannijecha/jecode@next
jecode
```

The npm package contains the native executable. It has no install scripts or
package dependencies; Node is only needed to use npm. For Linux x64, download
the native archive from [releases](https://github.com/giovannijecha/jecode/releases),
extract it and run `./jecode` in your project directory. macOS is not supported yet.

Or build from source with Rust 1.95.0 and a native linker:

```text
cargo build --locked --offline --release --bin jecode
```

Windows PowerShell, from the repository:

```powershell
.\target\release\jecode.exe
```

Linux:

```sh
./target/release/jecode
```

Follow the device sign-in instructions on first use. Subsequent runs reuse the
saved account and refresh access when needed. New conversations initially use
GPT-5.6 Luna with medium reasoning effort. `/model` shows models and effort
levels from the signed-in account; `--model MODEL --effort LEVEL` selects a pair
for one new conversation without changing saved defaults.
`jecode` uses your current directory; `--workspace PATH` selects another directory.
Use `jecode chat` for a conversation without file or command tools.
It still belongs to the directory where it was launched.
The workspace is the starting directory for relative paths. The default `local`
profile also accepts paths outside it; changes and commands execute directly.
Use `--access workspace` to restrict file tools to the selected directory.

```text
jecode login
jecode resume
jecode sessions
jecode --workspace PATH import-session V1_SESSION_ID
jecode logout
jecode --demo
```

`jecode login` signs in without creating a conversation. Esc or Ctrl+C cancels.
`jecode logout` removes only Jecode's local account access; it does not revoke
remote provider sessions. `sessions` and `resume` use the current directory, or
`--workspace PATH` when supplied. A session from another directory cannot be
resumed until you select its saved directory.
`import-session` creates a verified incremental copy of an older v1 session
without changing its source file.

`--demo` is an offline interface preview. Normal conversations connect to OpenAI
Account; Cargo's `--offline` flag only controls build dependency resolution.

## Working with Jecode

- Enter sends a message; during a response it queues guidance for the next model step.
- Esc interrupts work. Ctrl+Q exits and joins active work before closing.
- File changes show a diff. Commands show their shell, directory and timeout.
- On image-capable account models, `view_image` lets the model inspect a local PNG screenshot or a saved image ID.
- `/context` shows request size and available provider token counts.
- `/compact` summarizes earlier context while retaining the full saved history.
- Type `/` for a command menu; arrows select, Enter opens and Tab completes.
- `/login` and `/logout` change account access without closing the conversation.
- `/new`, `/resume`, `/model` and `/settings` manage conversations and preferences.
- `NO_COLOR` and reduced motion are supported.

See [usage and configuration](docs/USAGE.md), [session recovery](docs/SESSIONS.md), [tools](docs/TOOLS.md),
[architecture](docs/ARCHITECTURE.md), and [platform support](docs/COMPATIBILITY.md).

Commands run with your user permissions and are not sandboxed. Selected file
contents and command output may be sent to the model.
Credentials are ordinary private JSON files; see [security](SECURITY.md) and the
[TLS profile](docs/TLS.md) for the exact boundaries and limitations.

## Build quality

```text
cargo run --locked --offline --bin jecode-check -- check
```

This checks the complete dependency graph, source package inventory, formatting,
Clippy and offline tests.
Tests and CI are part of the public source. See [contributing](CONTRIBUTING.md)
and [releasing](docs/RELEASING.md).

Jecode aims to reduce redundant work while preserving correctness. Comparative
performance claims require representative, reproducible measurements.

MIT licensed. Copyright 2026 Giovanni Jecha.
