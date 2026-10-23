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
under `target` and loopback HTTP. They exercise installed curl 8.3+ and Bash without
contacting OpenRouter or reading personal configuration.

## Run a focused check

Pass a test-name filter after the Cargo options:

```text
cargo test --locked --offline agent::routing_tests::
cargo test --locked --offline agent::backend_tests::
cargo test --locked --offline backend::openrouter::calls::tests::
cargo test --locked --offline tools::processes::tests::
cargo test --locked --offline sessions::journal_tests::
cargo test --locked --offline checkpoint
```

Check the reported test count: a successful command that matched no tests has not
verified the change. Use `-- --list` to inspect available names.

| Change | Relevant checks |
| --- | --- |
| Agent/context/backend | `agent::backend_tests` uses two synthetic protocols across tool execution, compaction, save and resume. `backend/openrouter/` tests use loopback responses for the adapter. |
| Files/processes | Tests beside `tools/` and `process/`; retain timeout, cancellation and owner-loss behavior. |
| Sessions/attachments | Tests beside `sessions/` and `attachments/`; use isolated storage and check recovery. |
| Checkpoints | Tests beside `checkpoints/` and integration tests in agent, session, tools and TUI. Cover per-call preimages, no startup acquisition, scoped Bash, partial/unreadable/reparse scope, Unicode/aliases, corruption, cancellation, cross-process locks and process death, fixed 24-hour expiry, conflict checks and precise partial undo. |
| TUI | State/layout/VT screen tests in `tui/`, plus the platform's native terminal fixture. |

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
