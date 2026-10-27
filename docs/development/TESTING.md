# Development and testing

Run these commands from the repository root with Rust 1.95.0, pinned by
`rust-toolchain.toml`. Install the native tools listed in the [README](../../README.md#install).

## Windows toolchain

MSVC builds need the Visual C++ linker on PATH. To use the installed GNU LLVM
toolchain instead, open PowerShell at the repository root and select its bundled
linker for this terminal:

```powershell
$env:RUSTUP_TOOLCHAIN = '1.95.0-x86_64-pc-windows-gnullvm'
$taskSysroot = rustc --print sysroot
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER = "$taskSysroot\lib\rustlib\x86_64-pc-windows-gnullvm\bin\rust-lld.exe"
```

This requires the pinned GNU LLVM toolchain with its `rust-mingw` component.
The [Windows installer](../../README.md#windows) selects it automatically when
available; that selection applies only to the install command.

## Normal checks

```text
cargo build --locked --offline
cargo fmt --all -- --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline
```

All four commands must exit successfully. Ignored tests require an explicit
invocation; a normal test run does not establish their coverage.

Tests use original Rust fixtures, synthetic credentials, isolated directories
under `target` and loopback HTTP. They exercise installed curl 8.3+ and Bash
without contacting OpenRouter or OpenAI and without reading personal
configuration.

Unix owner-loss tests also use the system `ps` command to inspect process groups.
They synchronize owner exit with live descendants, then wait for the group to
stop; zombies awaiting reaping do not count as running commands.

OpenAI fixtures also use isolated account records and loopback HTTP. They do
not read Codex credentials or launch a real ChatGPT sign-in. Native signature
checks exercise Windows .NET or the Unix `openssl` executable. A successful
fixture run does not establish account eligibility or real plan inference.

Opt-in live OpenAI checks use the selected saved ChatGPT registration and
synthetic inputs. They consume ChatGPT plan usage, never print tokens, and stay
outside ordinary offline verification:

```text
cargo test --locked --offline backend::openai::live::inspect_live_model_and_stream -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline backend::openai::live::live_tool_roundtrip_uses_context_and_selected_effort -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline backend::openai::live::live_image_and_tool_roundtrip -- --exact --ignored --nocapture --test-threads=1
```

## Run a focused check

Pass a test-name filter after the Cargo options:

```text
cargo test --locked --offline agent::routing_tests::
cargo test --locked --offline agent::backend_tests::
cargo test --locked --offline backend::openrouter::calls::tests::
cargo test --locked --offline auth::openai::
cargo test --locked --offline backend::openai::
cargo test --locked --offline tools::processes::tests::
cargo test --locked --offline tools::exec::tests::
cargo test --locked --offline tools::interactive::tests::
cargo test --locked --offline sessions::journal_tests::
cargo test --locked --offline checkpoint
```

Check the reported test count: a successful command that matched no tests has not
verified the change. Use `-- --list` to inspect available names.

| Change | Relevant checks |
| --- | --- |
| Agent/context/backend | `agent::backend_tests` uses two synthetic protocols across tool execution, compaction, save and resume. Both `backend/openrouter/` and `backend/openai/` tests use loopback responses for their adapters. |
| Files/processes | Tests beside `tools/` and `process/`; retain timeout, cancellation and owner-loss behavior. |
| Sessions/attachments | Tests beside `sessions/` and `attachments/`; use isolated storage and check recovery. |
| Checkpoints | Tests beside `checkpoints/` and integration tests in agent, session, tools and TUI. Cover per-call preimages, no startup acquisition, scoped Bash, partial/unreadable/reparse scope, Unicode/aliases, corruption, cancellation, cross-process locks and process death, fixed 24-hour expiry, conflict checks and precise partial undo. |
| TUI | State/layout/VT screen tests in `tui/`, plus the platform's native terminal fixture. |
| Browser | `browser::` tests use loopback DevTools fixtures for waits, log paging, eviction and reconnection, plus the installed-browser fixtures below. |

Installed Chrome or Edge fixtures serve a loopback site and use isolated
profiles under `target` or ignored `.tmp/`. Run them one at a time after
browser changes; the last one briefly opens a visible window:

```text
cargo test --locked --offline browser::tests::live_chromium_localhost_interaction_and_profile_persistence -- --exact --ignored --test-threads=1
cargo test --locked --offline browser::live_tests::live_chromium_viewport_dimensions_and_validation -- --exact --ignored --test-threads=1
cargo test --locked --offline browser::live_tests::live_chromium_snapshot_semantics_logs_and_wait_diagnostics -- --exact --ignored --test-threads=1
cargo test --locked --offline browser::snapshot::tests::live_chromium_snapshot_values_text_and_refs -- --exact --ignored --test-threads=1
cargo test --locked --offline browser::live_tests::live_chromium_visible_viewport -- --exact --ignored --test-threads=1
```

Work chapters have focused card, state and viewport tests, including a 10,000-call
replay and latest-turn expansion. To paint the actual compact and expanded cards
at 80 and 40 cells with synthetic events:

```text
cargo test --locked --offline tui::specimen::work_chapters -- --ignored --nocapture
```

The [native terminal fixture](PLATFORMS.md#windows) also checks compact chapter
facts and existing-card expansion through Ctrl+O.

Run the applicable normal checks after focused verification. Keep credentials,
logs, screenshots and temporary drivers out of tracked files; use ignored `.tmp/`
or `target` directories.

Focused recovery checks run without a provider:

```text
cargo test --locked --offline checkpoint
```

Storage subprocess fixtures terminate a writer or reader and verify released OS
locks, retained live reads and cleanup of expired pending data. Normal tests use
isolated homes; no real archive is read or deleted. Free-space probes have
separate cancellation, insufficient-space and failed-helper checks.

The opt-in matched tool workload uses release binaries with identical fixtures:

```text
cargo test --locked --offline --release checkpoint_bench::matched_tools -- --exact --ignored --nocapture --test-threads=1
```

Set `JECODE_BENCH_CASE` to `write4k`, `edit1m`, `bash1`, `bash100`, `bash1000`,
`restore1`, `restore100`, `restore1000` or `large256m`; set `JECODE_BENCH_COUNT`
for measured samples. Two warmups precede them. `JECODE_BENCH_ENABLED=0` disables
capture for control cases. Exact output validation and restore setup are outside
the timed interval. Run variants serially with no builds/CI in progress and
alternate their order. See [performance](PERFORMANCE.md) for evidence and limits.

## Development binary and installation

```text
cargo run --locked --offline -- --help
cargo run --locked --offline
```

The first command checks CLI startup without configuration. The second opens the
application with normal user configuration. To work in a different project,
invoke the built executable there by its absolute path.

To check the Windows installer without changing PATH, choose a disposable install
root in PowerShell:

```powershell
.\INSTALL.ps1 -InstallRoot "$PWD\.tmp\install-check" -NoPath
& "$PWD\.tmp\install-check\bin\jecode.exe" --version
```

The Unix installer accepts an absolute root: `sh INSTALL.sh /absolute/test/root`.
Both installers build offline and print the installed executable's version.

## Native and manual checks

Interactive execution tests use original Rust fixtures and isolated native
terminals. Unix PTY tests require `cc` on first use, plus `/proc` on Linux or
`/bin/ps` on macOS for session cleanup. The compiled helper cache is reused
across test runs under `target/measurements/pty-cache`, keyed by platform and
source contents. They check input, resize, interrupt, owner loss, job-control groups,
saved output and per-control checkpoints. Password-entry checks use synthetic
values with the target's echo disabled.

Unix interrupt probes also use system `stty` and `od` to check synthetic bytes,
terminal modes and queued input. They cover raw input, remapped or disabled
`VINTR`, `NOFLSH`, delayed input frames, saturated connections and input/resize
after a client detaches its controlling terminal.
The Linux-only blocked raw-input probe also uses system `tty` and `kill`.
The `TIOCNOTTY` detachment fixture is Linux-only; both Unix platforms check
no-foreground handling on a newly opened PTY before terminal acquisition.

The Windows direct-input fixture drives the real Rust renderer and verifies
that local terminal input, including emoji, stays outside conversation history:

```text
cargo test --locked --offline tui::terminal::windows::native_tests::native_direct_input::native_windows_direct_input -- --exact --ignored --nocapture --test-threads=1
```

[Platform verification](PLATFORMS.md) contains native ConPTY/PTY commands,
local coverage and physical host checks. Use those fixtures for input, resize,
terminal restoration and owner-loss changes. They use synthetic conversations.

Windows also has interactive smoke fixtures. Run one at a time in a real console:

```text
cargo test --locked --offline tui::manual_tests::composer_windows_smoke -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline tui::manual_tests::resize_windows_smoke -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline tui::persistence_smoke::sessions_windows_smoke -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline tui::long_work_smoke::long_work_windows_smoke -- --exact --ignored --nocapture --test-threads=1
```

Follow each fixture's printed prompts; these tests wait for keyboard interaction.
Do not run private child fixtures directly. Opt-in timing probes are documented
in [performance](PERFORMANCE.md).

## CI

[CI.yml](../../.github/workflows/CI.yml) runs the normal checks and then the
platform's exact, ignored native terminal fixture from [platform
verification](PLATFORMS.md). It does not run the other ignored tests, which
include manual interaction, private child fixtures, live-provider evaluation and
opt-in measurements. Windows and Linux use maintainer-hosted local runners;
macOS uses GitHub's hosted runner. The required checks are `Rust (Windows)`,
`Rust (Linux)` and `Rust (macOS)`.

Local runners use fresh checkouts and one-job credentials, with a host policy
rejecting fork code before workflow steps. Their jobs queue while the maintainer's
PC is offline. A queued or skipped job is not a successful verification.
