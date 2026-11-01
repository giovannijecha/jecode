# Development and testing

Run every command on this page from the repository root. Rust 1.95.0 is pinned
by `rust-toolchain.toml`, so `rustup` selects it automatically. The build
prerequisites for each platform are listed in
[Installation](../usage/INSTALLATION.md#build-prerequisites).

## Windows toolchain

The pinned toolchain uses your default host. With the MSVC host, the Visual C++
linker must be on `PATH`. To use the GNU LLVM toolchain and its bundled linker
instead, run this once per PowerShell session:

```powershell
rustup toolchain install 1.95.0-x86_64-pc-windows-gnullvm --component rust-mingw --component rustfmt --component clippy
$env:RUSTUP_TOOLCHAIN = '1.95.0-x86_64-pc-windows-gnullvm'
$sysroot = rustc --print sysroot
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER = "$sysroot\lib\rustlib\x86_64-pc-windows-gnullvm\bin\rust-lld.exe"
```

CI uses this toolchain on Windows.

## Normal checks

```text
cargo build --locked --offline
cargo fmt --all -- --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline
```

All four must succeed before a change is delivered. The test suite uses original
Rust fixtures, synthetic credentials, isolated directories under `target` and
loopback HTTP. It exercises the native HTTP adapter and process boundaries
without contacting a provider or reading your configuration.

Ignored tests run only when named explicitly; a normal run says nothing about
them. They are the native terminal fixtures, live-provider checks, manual smoke
tests and measurements described below.

## Focused checks

Pass a test-name filter after the Cargo options:

```text
cargo test --locked --offline checkpoint
cargo test --locked --offline tools::discovery
cargo test --locked --offline agent::backend_tests::
```

Check the reported count: a command that matched no tests verified nothing. Use
`cargo test --locked --offline -- --list` to see available names, and add
`--ignored` to list the opt-in tests.

| Area | Where the tests are |
| --- | --- |
| Agent loop, context and backends | `agent::backend_tests` drives two synthetic protocols through tools, compaction, save and resume. `backend::openrouter`, `backend::openai` and `backend::anthropic` use loopback servers for their adapters. |
| Accounts | `auth::openai`, `auth::anthropic`, `auth::accounts`. |
| Routing | `agent::routing_tests`, `backend::openrouter`. |
| Tool scheduling | `agent::execution::scheduling`, `agent::execution::declared`, `sessions::tool_batch_tests`. |
| Files and discovery | `tools::discovery`, `path_filter`, `agent::discovery_tests`. |
| Processes and terminals | `tools::processes`, `tools::exec`, `tools::interactive`, `process`. |
| Project map | `project_map`, `agent::map_tests`. |
| Collaboration | `agent::collaboration`, `collaboration::workspace`, `session::agents`, `tui::agents`. |
| Sessions and attachments | `sessions`, `attachments`. |
| Checkpoints | `checkpoint` matches the store and its agent, session, tool and TUI integration tests. |
| TUI | State, layout and VT screen tests under `tui`, plus the native terminal fixture for your platform. |
| Browser | `browser` uses loopback DevTools fixtures; installed-browser checks are below. |

Keep credentials, logs, screenshots and temporary drivers out of tracked files;
use the ignored `.tmp/` or `target` directories.

## Native terminal fixtures

These drive the real renderer through a pseudo-terminal and check input,
resize, quitting and terminal restoration after normal exit and owner loss. CI
runs the one for each platform. Run them after the normal suite, one at a time;
running them concurrently with other suites can exceed their wait limits.

```text
cargo test --locked --offline tui::terminal::windows::native_tests::native_windows_fullscreen_controls_and_terminal_lifetime -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline tui::terminal::windows::native_tests::native_direct_input::native_windows_direct_input -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline tui::terminal::unix::native_tests::native_linux_tui_controls_and_terminal_lifetime -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline tui::terminal::unix::native_tests::native_macos_tui_controls_and_terminal_lifetime -- --exact --ignored --nocapture --test-threads=1
```

The first two need Windows 10 or later with ConPTY; the Linux fixture needs the
util-linux `script` tool; the macOS fixture needs the system `script` and `tty`
utilities. Do not run the private `native_pty_fixture` child directly. Other
Windows fixtures cover permission requests
(`native_tests::native_permissions::native_windows_permissions`) and light
terminal backgrounds (`native_tests::native_background::`).

Interactive `exec` tests need `cc` on first use on Unix, plus `/proc` on Linux
or `/bin/ps` on macOS. The compiled helper is cached under
`target/measurements/pty-cache`. Unix interrupt probes also use the system
`stty` and `od`, and owner-loss tests use `ps`.

## Windows command confinement

The normal suite launches an original Rust probe inside the Workspace
confinement and checks permitted and denied file operations, inherited
confinement in descendants and rejection of interactive terminals. PowerShell
and .NET compatibility checks are opt-in because Windows cannot start some
programs inside the confinement from a service session (see
[Permissions](../usage/PERMISSIONS.md#platform-limits)). Run them from an
interactive user session:

```text
cargo test --locked --offline agent::execution::sandbox_tests::workspace_powershell_uses_installed_rust_tools_and_git_status -- --exact --ignored --test-threads=1
cargo test --locked --offline process::windows::sandbox_tests::workspace_native_powershell_runs_without_user_profile -- --exact --ignored --test-threads=1
cargo test --locked --offline process::windows::sandbox_tests::workspace_native_child_and_grandchild_enforce_file_policy -- --exact --ignored --test-threads=1
cargo test --locked --offline process::windows::sandbox_tests::workspace_native_loopback_client_and_server -- --exact --ignored --test-threads=1
cargo test --locked --offline tools::interactive::tests::windows::workspace_sandbox_runs_native_exec_and_rejects_interactive_terminal -- --exact --ignored --test-threads=1
```

## Installed-browser checks

These use an installed Chrome or Edge with isolated profiles under `target` or
`.tmp/` and a loopback site; no model request is made. Run them one at a time
after browser changes. The last one briefly opens a visible window.

```text
cargo test --locked --offline agent::browser_tests::native_browser_tool_delivers_images_and_preserves_only_named_profile_state -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline browser::tests::live_chromium_localhost_interaction_and_profile_persistence -- --exact --ignored --test-threads=1
cargo test --locked --offline browser::live_tests::live_chromium_viewport_dimensions_and_validation -- --exact --ignored --test-threads=1
cargo test --locked --offline browser::live_tests::live_chromium_snapshot_semantics_logs_and_wait_diagnostics -- --exact --ignored --test-threads=1
cargo test --locked --offline browser::snapshot::tests::live_chromium_snapshot_values_text_and_refs -- --exact --ignored --test-threads=1
cargo test --locked --offline browser::live_tests::live_chromium_visible_viewport -- --exact --ignored --test-threads=1
```

`http::tests::trusted_https_uses_os_tls_policy` checks the native TLS boundary
against `https://example.com/` without credentials.

## Live provider checks

These make real requests with a saved account and are never part of offline
verification. They print no tokens.

ChatGPT checks use the selected saved ChatGPT account and consume plan usage:

```text
cargo test --locked --offline backend::openai::live::inspect_live_model_and_stream -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline backend::openai::live::live_tool_roundtrip_uses_context_and_selected_effort -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline backend::openai::live::live_image_and_tool_roundtrip -- --exact --ignored --nocapture --test-threads=1
```

Claude checks require an explicit absolute `JECODE_HOME` containing a
signed-in test account. They verify that extra usage is disabled before any
inference, use synthetic messages, make one request (stream) or two (tool round
trip) and bypass automatic retries:

```text
cargo test --locked --offline backend::anthropic::live::minimal_subscription_stream -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline backend::anthropic::live::subscription_tool_roundtrip -- --exact --ignored --nocapture --test-threads=1
```

The cache-reuse checks also require an explicit absolute `JECODE_HOME`. This
filter runs both the Claude check (four requests) and the ChatGPT check (three
requests) and prints token and cache counters:

```text
cargo test --locked --offline efficiency_live::durable_prefix_cache_survives_runtime_state_changes -- --ignored --nocapture --test-threads=1
```

`agent::compaction::live_tests::live_summary_and_resumed_readback_preserve_critical_work`
makes two Claude requests to check that a compaction summary keeps critical
task facts across a resume. Results are recorded in
[Performance](PERFORMANCE.md#subscription-context-reuse).

## Manual checks

Windows has interactive smoke fixtures that wait for keyboard input. Run one at
a time in a real console and follow the printed prompts:

```text
cargo test --locked --offline tui::manual_tests::composer_windows_smoke -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline tui::manual_tests::resize_windows_smoke -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline tui::persistence_smoke::sessions_windows_smoke -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline tui::long_work_smoke::long_work_windows_smoke -- --exact --ignored --nocapture --test-threads=1
```

The `tui::specimen::` tests paint real cards and panels with synthetic events,
for example `tui::specimen::chapter_live::chapter_live` or
`tui::specimen::discovery::discovery_cards`. Run them with
`-- --exact --ignored --nocapture` to inspect the output.

For checks that need a real terminal host, such as physical input, fonts and
clipboard delivery, use the checklist in
[Platform support](PLATFORMS.md#check-a-terminal-host).

## Development binary and installers

```text
cargo run --locked --offline -- --help
cargo run --locked --offline
```

The first command checks CLI startup without configuration; the second opens
Jecode with your normal user configuration in the repository directory. To work
in another project, run the built executable from that project by its absolute
path, or set `JECODE_HOME` to an absolute scratch directory to keep your real
configuration untouched.

The installers only copy a prebuilt executable; they never build. To check the
Windows installer without changing `PATH`, prepare a package and install it into
a disposable root:

```powershell
.\PACKAGE.ps1
$package = Get-ChildItem target\package -Directory | Select-Object -First 1
& "$($package.FullName)\INSTALL.ps1" -InstallRoot "$PWD\.tmp\install-check" -NoPath
& "$PWD\.tmp\install-check\bin\jecode.exe" --version
```

On Linux or macOS, run `sh PACKAGE.sh`, then
`sh target/package/jecode-<platform>-<arch>/INSTALL.sh /absolute/test/root`.

## npm distribution checks

Node.js 22 or later and npm are needed for these distribution checks, not for
the standalone Rust executable. From the repository root, test the launcher:

```text
node --test npm/launcher.test.cjs
node --test tools/npm/release.test.mjs
```

The [release runbook](RELEASING.md) describes staging and packing the platform
binaries. After preparing a tarball, replace
`path/to/giovannijecha-jecode-0.1.1.tgz` below
with its actual path and run the lifecycle fixture on each supported platform:

```text
node tools/npm/lifecycle.mjs path/to/giovannijecha-jecode-0.1.1.tgz
```

The fixture uses an isolated npm installation, local registry, and user home
to exercise real installation, update, removal, and native `--version`
execution without touching your normal installation or `~/.jecode` data.
These checks do not publish a package.

The npm terminal fixtures are opt-in. After packing locally, set
`JECODE_NPM_LAUNCHER` to the absolute path of
`target/npm-local/package/bin/jecode.cjs` and `JECODE_NPM_NODE` to Node's
absolute `process.execPath`. Run the fixture for the current platform:

```text
cargo test --locked --offline tui::terminal::windows::native_tests::native_npm::npm_launcher_native_tui -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline tui::terminal::unix::native_tests::npm_tests::npm_launcher_native_tui -- --exact --ignored --nocapture --test-threads=1
```

The second command applies to Linux and macOS. CI supplies both paths and runs
the matching fixture through the packaged launcher and real terminal bridge.

## CI

[`CI.yml`](https://github.com/giovannijecha/jecode/blob/main/.github/workflows/CI.yml)
runs formatting, the offline build, all-target Clippy and the test suite, then
the platform's native terminal fixtures (on Windows, also the direct-input
fixture). It also builds a release executable, tests npm packaging and the
install/update/uninstall lifecycle, and runs the opt-in native terminal fixture
through the npm launcher. The required checks are
`Rust (Windows)`, `Rust (Linux)` and `Rust (macOS)`.

Windows and Linux jobs run on maintainer-hosted runners with fresh checkouts and
one-job credentials; the host rejects fork code before workflow steps run.
These jobs queue while the maintainer's machine is offline, and a queued or
skipped job is not a successful verification. macOS runs on GitHub's hosted
`macos-15` runner.
