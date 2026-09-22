# Jecode

An owned coding harness for your terminal, written in Rust.

Jecode streams model responses, reads a selected workspace, proposes file changes,
and runs commands after approval. Conversations live in the terminal's natural
scrollback. Sessions, settings and account credentials belong to your user profile.

The application uses the Rust standard library and original code, with no external
crates, vendored libraries or runtime helpers for networking and presentation.
One controller owns the conversation and orders tool effects.

## Try it

Jecode is an **alpha** for Windows and Linux. Build from source with the pinned
Rust 1.95.0 toolchain and a native linker:

```text
cargo build --locked --offline --release --bin jecode
```

Windows PowerShell, from the repository:

```powershell
.\target\release\jecode.exe --account --workspace .
```

Linux:

```sh
./target/release/jecode --account --workspace .
```

Follow the device sign-in instructions on first use. Subsequent runs reuse the
saved account and refresh access when needed. GPT-5.6 Luna with medium effort is
the default; `--model gpt-5.6-terra` selects Terra for that run.
Without `--workspace`, Jecode has no file or command tools.

```text
jecode --sessions
jecode --resume SESSION_ID
jecode --logout
jecode --demo
```

`--demo` is an offline interface preview. `--account` connects to OpenAI Account;
Cargo's `--offline` flag only controls build dependency resolution.

## Working with Jecode

- Enter sends a message; during a response it queues guidance for the next model step.
- Esc interrupts work. Ctrl+Q exits and joins active work before closing.
- File changes show a diff. Commands show their shell, directory and timeout.
- `/context` shows request size and available provider token counts.
- `/compact` summarizes earlier context while retaining the full saved history.
- Tab completes local slash commands. `NO_COLOR` and reduced motion are supported.

See [usage and configuration](docs/USAGE.md), [tools](docs/TOOLS.md),
[architecture](docs/ARCHITECTURE.md), and [platform support](docs/COMPATIBILITY.md).

Commands run with your user permissions: approval is not an operating-system
sandbox. Selected file contents and command output may be sent to the model.
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
