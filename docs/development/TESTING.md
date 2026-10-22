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
under `target` and loopback HTTP. They exercise installed curl and Bash without
contacting OpenRouter or reading personal configuration.

## Run a focused check

Pass a test-name filter after the Cargo options:

```text
cargo test --locked --offline agent::routing_tests::
cargo test --locked --offline tools::processes::tests::
cargo test --locked --offline sessions::journal_tests::
cargo test --locked --offline checkpoint
```

Check the reported test count: a successful command that matched no tests has not
verified the change. Use `-- --list` to inspect available names.

| Change | Relevant checks |
| --- | --- |
| Agent/context/provider | Tests beside `agent/`, `context/` and `openrouter/`; use loopback responses. |
| Files/processes | Tests beside `tools/` and `process/`; retain timeout, cancellation and owner-loss behavior. |
| Sessions/attachments | Tests beside `sessions/` and `attachments/`; use isolated storage and check recovery. |
| Checkpoints | Tests beside `checkpoints/` and the `checkpoint_tests` modules in `agent/`, `session/`, `tools/` and `tui/`. Cover total and active-generation admission, four processes sharing a reduced test quota, selective discovery, polling coverage, long-turn pruning, idle-reader cleanup, retries after points or unpins, crash recovery, conflicts and exact undo. Children re-enter the test binary with isolated archive homes; reduced test ceilings have no production configuration path. |
| TUI | State/layout/VT screen tests in `tui/`, plus the platform's native terminal fixture. |

Run the applicable normal checks after focused verification. Keep credentials,
logs, screenshots and temporary drivers out of tracked files; use ignored `.tmp/`
or `target` directories.

The combined checkpoint regression runs in the ordinary suite. To isolate it:

```text
cargo test --locked --offline checkpoints::tests::process::stress::four_sessions_survive_concurrent_collection_and_crashed_restore_near_capacity -- --exact --nocapture --test-threads=1
```

It places an isolated archive 4 MiB below the real 1 GiB logical limit, with a
4 MiB retention policy and four writer processes. The fourth writer commits
while collection is paused inside its copy. The fixture kills the collector
after its durable move but before `CURRENT` is updated, then kills a restore
after its first file installation. Reopening must preserve the concurrent
write, repair the pointer, classify pending work and support exact undo. Stage
barriers exist only under `cfg(test)`; the filler uses file length, not a GiB
of generated contents. Capacity and retention pruning are asserted separately.

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
