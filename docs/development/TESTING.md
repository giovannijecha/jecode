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

CI and release preparation run the complete suite with `-- --test-threads=4`
to bound concurrent native processes. Use the same option for a matching local
run. Individual native terminal fixtures retain `--test-threads=1`.

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
cargo test --locked --offline tools::files
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
| MCP | `mcp::` covers protocol, storage and OAuth with isolated loopback servers; `agent::mcp_tests` checks model-to-server dispatch, session reuse and redaction; `tui::mcp::` covers native panels and async configuration. |
| LSP | `lsp::` uses an original child-process server for framing, synchronization, capabilities and lifecycle. `agent::lsp_tests` covers model dispatch at all local permission levels; `process::duplex` covers persistent I/O and descendant cleanup. `tui::lsp` and card tests cover the LSP panels and cards. |
| Routing | `agent::routing_tests`, `backend::openrouter`. |
| Tool scheduling | `agent::execution::scheduling`, `agent::execution::declared`, `sessions::tool_batch_tests`. |
| Files and discovery | `tools::discovery`, `path_filter`, `agent::discovery_tests`. |
| Processes and terminals | `tools::processes` (lifecycle, yielding, completions), `agent::process_delivery_tests` (delivery and idle continuation), `session::processes`, `tui::processes`, `tools::exec`, `tools::interactive`, `process`. |
| Collaboration | `agent::collaboration`, `collaboration::workspace`, `session::agents`, `tui::agents`. |
| Sessions and attachments | `sessions`, `attachments`. |
| TUI | State, layout and VT screen tests under `tui`, plus the native terminal fixture for your platform. |
| Browser | `browser` uses loopback DevTools fixtures; installed-browser checks are below. |

Keep credentials, logs, screenshots and temporary drivers out of tracked files;
use the ignored `.tmp/` or `target` directories.

## Offline long-context checks

[Context checks](CONTEXT_CHECKS.md) describes one opt-in, provider-free probe:
read-only replay of the saves in current-format session journals. It validates
context serialization and projection before spending live model calls. It does
not establish model quality, provider cache behavior or ten-hour live
performance.

## Native terminal fixtures

These drive the real renderer through a pseudo-terminal and check input,
resize, quitting and terminal restoration after normal exit and owner loss. CI
runs the fixtures for each platform. Run them after the normal suite, one at a time;
running them concurrently with other suites can exceed their wait limits.
Windows screen snapshots compile the owned C# probe once per fixture, then run
the executable directly. Repeated snapshots keep the same native cell and mode
assertions without repeatedly starting PowerShell and its compiler. Timeout
diagnostics retain the end of the terminal output so child failures stay visible.

```text
cargo test --locked --offline tui::terminal::windows::native_tests::native_windows_fullscreen_controls_and_terminal_lifetime -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline tui::terminal::windows::native_tests::native_direct_input::native_windows_direct_input -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline tui::terminal::windows::native_tests::native_lsp::native_windows_lsp_panels -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline tui::terminal::windows::native_tests::native_processes::native_windows_process_panels -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline tui::terminal::windows::native_tests::native_active_history::native_windows_escape_during_active_long_history -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline tui::terminal::unix::native_tests::native_linux_tui_controls_and_terminal_lifetime -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline tui::terminal::unix::native_tests::native_macos_tui_controls_and_terminal_lifetime -- --exact --ignored --nocapture --test-threads=1
```

The Windows fixtures need Windows 10 or later with ConPTY; the Linux fixture needs the
util-linux `script` tool; the macOS fixture needs the system `script` and `tty`
utilities. Do not run the private `native_pty_fixture` child directly. Other
Windows fixtures cover permission requests
(`native_tests::native_permissions::native_windows_permissions`) and the inks chosen
on light terminals, with no background painted (`native_tests::native_background::`). The fullscreen
fixture also paints tool cards through the console, including multi-file edits
and the steps of one `steps` call.

Linux and macOS run the same conversations, from `tui::terminal::fixtures`, through
one filter: permissions, direct terminal input, startup with and without a provider,
the startup screen, light inks chosen through OSC 11, and a terminal that
only declares the 256-colour palette while speaking the keyboard protocol:

```text
cargo test --locked --offline tui::terminal::unix::native_tests::scenarios::native_unix -- --ignored --nocapture --test-threads=1
```

The tool card, authentication, MCP and LSP panels have no Unix fixture of their own.
Their Windows fixtures read cells back from the console to check ConPTY's translation,
which Unix terminals do not have; the shared rendering tests cover their content.

The active-history fixture resumes synthetic history, executes a local read and
holds a loopback model stream open. It exercises typing, resizing and Esc through
ConPTY, then verifies that the draft survives and the terminal is restored. It
uses isolated configuration and no provider account or external model request.

The MCP fixture uses the real Windows renderer at 44 and 100 columns, covering
the list, add, form and Advanced panels (including unchecked and signed-out
states), successful calls and uncertain remote outcomes:

```text
cargo test --locked --offline tui::terminal::windows::native_tests::native_mcp::native_windows_mcp_panels -- --exact --ignored --nocapture --test-threads=1
```

An explicit live MCP OAuth check opens the selected provider in the browser,
registers or identifies Jecode, and waits for the user's approval. It discovers
tools, then refreshes and discovers again when the server issues a refresh token.
Credentials use an isolated fixture store removed when the test finishes. It
does not read personal Jecode configuration or execute service mutations.

```powershell
$env:JECODE_MCP_LIVE_PROVIDER = 'vercel'
cargo test --locked --offline mcp::auth::live::provider_oauth_roundtrip -- --exact --ignored --nocapture --test-threads=1
```

Choose `github`, `supabase`, `cloudflare` or `vercel`. GitHub also checks that
the Actions toolset is visible. On Windows on 2026-11-03 all four completed
authorization, refresh and discovery; Vercel returned 241 tools, Supabase 31,
Cloudflare 4 and GitHub 48. Resource access and remote mutations were not tested.

On Unix, `cc` builds the interactive `exec` helper during the Cargo build, and the
test suite uses it again for strict-warning and PTY contract checks. Under test the
helper installs to `.tmp/native-tests`. Job-control cleanup uses `/proc` on Linux and
`/bin/ps` on macOS. Unix interrupt probes also use the system `stty` and `od`, and
owner-loss tests use `ps`.

## Opt-in language-server interoperability

Ordinary LSP tests compile an original Rust fixture server and require no
installed language server. The ignored interoperability tests use executables
explicitly selected by the developer, never download them and never read the
user's LSP configuration. They create isolated projects and configuration.

For example, in PowerShell after the normal toolchain setup:

```powershell
$env:JECODE_LSP_RUST = 'C:\Tools\rust-analyzer.exe'
$env:JECODE_LSP_CLANGD = 'C:\Tools\clangd.exe'
cargo test --locked --offline tools::lsp::live_tests -- --ignored --nocapture --test-threads=1
```

On Unix set the same two environment variables to the selected executables.
Use rust-analyzer compatible with the pinned Rust 1.95 toolchain. An official
toolchain component may need its compiler library directory on the server's
PATH; `JECODE_LSP_RUST_PATH` overrides PATH for that test server only.
`JECODE_LSP_RUST_TOOLCHAIN` selects the test server's Cargo toolchain, default
`1.95.0`. These variables do not configure Jecode's application defaults.

The Rust fixture disables sysroot source loading, build scripts and proc macros
and uses offline Cargo checks; it does not install rust-src. Cargo diagnostics
and analyzer diagnostics may arrive through different push/pull channels.
The C++ fixture supplies explicit compile commands and disables background
indexing. Both tests cover navigation, references, an introduced and updated
error, a warning, clearing diagnostics, external edits and unopened dependencies,
spaces, non-BMP text,
CRLF and instance reuse. Their timings describe these small fixtures only.
Record actual server versions and executed platforms when reporting results.

## Command confinement

The normal suite launches an original Rust probe inside the Workspace
confinement and checks permitted and denied file operations, inherited
confinement in descendants, and rejection of interactive terminals. On Linux and
macOS, `sandbox::unix_tests` also runs the system shell, `cc`, Git and any of
`cargo`, `node`, `npm` and `python3` the host has. On a kernel without Landlock they
skip with a printed reason, which the test harness hides when a test passes. CI sets
`JECODE_REQUIRE_CONFINEMENT` so that a job on such a kernel fails instead of staying
green with nothing checked. `sandbox::unix_escape_tests` covers routes around the
policy: loader variables and, on macOS, hard links, clones, other processes'
environments and signals. `native_unix_confined_commands_have_no_terminal` needs a
real controlling terminal, so it runs with the native conversations.

On Windows it also checks drive-root listing only for an installed Node.js
runtime. PowerShell and .NET
compatibility checks are opt-in because Windows cannot start some programs
inside the confinement from a service session (see
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
cargo test --locked --offline browser::keyboard_live_tests -- --ignored --nocapture --test-threads=1
cargo test --locked --offline browser::live_tests::live_chromium_viewport_dimensions_and_validation -- --exact --ignored --test-threads=1
cargo test --locked --offline browser::live_tests::live_chromium_snapshot_semantics_logs_and_wait_diagnostics -- --exact --ignored --test-threads=1
cargo test --locked --offline browser::snapshot::tests::live_chromium_snapshot_values_text_and_refs -- --exact --ignored --test-threads=1
cargo test --locked --offline browser::live_tests::live_chromium_visible_viewport -- --exact --ignored --test-threads=1
```

The keyboard fixtures exercise printable keys and Enter in a textarea and form,
select-all/delete/undo/redo, reverse Tab, Unicode contenteditable input, a
single `type` sequence that follows Enter/Tab focus changes between grid cells,
1,000 keys within the default timeout, widgets that manage their own focus,
keys on buttons, checkboxes, modal dialogs and selections, and a `confirm` opened
by a key and then answered or closed from another controller. A key-up dialog
also verifies that partial-input diagnostics describe the current key's effects.
They inspect trusted DOM events and the resulting values and focus. CI and
release preparation run these checks on all four targets using installed Chrome
or Edge. The shared `tools/ci-chrome.sh` wrapper uses the runner's outer sandbox
because nested Chrome sandbox namespaces are disabled there; ordinary browser
launches are unchanged.

`http::tests::trusted_https_uses_os_tls_policy` accepts `https://example.com/`
and rejects `https://self-signed.badssl.com/` without account credentials. macOS
CI and release preparation run it explicitly to verify the native TLS boundary.

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

To check the package scripts without building or installing, run
`node --test tools/package.test.mjs`. It uses an isolated source tree with stub
build commands and verifies the document exclusions and existing-output refusal.

## npm distribution checks

Node.js 22 or later and npm are needed for these distribution checks, not for
the standalone Rust executable. From the repository root, test the launcher:

```text
node --test npm/launcher.test.cjs
node --test tools/npm/release.test.mjs
```

The [release runbook](RELEASING.md) describes staging and packing the platform
binaries. After preparing a tarball, replace the path below with the actual
tarball path and `<version>` with the package version from `Cargo.toml`. Run the
lifecycle fixture on each supported platform:

```text
node tools/npm/lifecycle.mjs path/to/giovannijecha-jecode-<version>.tgz
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

## OS floors

[Platform support](PLATFORMS.md#supported-versions) lists the floors. To reproduce
a Linux release that meets its floor, link against the Debian 10 sysroot. This
needs `curl`, `sha256sum`, `dpkg-deb` and `cc`, and keeps its own target directory
because Cargo does not track the compiler wrapper's contents:

```sh
eval "$(sh tools/linux-sysroot.sh target/sysroot | sed 's/^/export /')"
CARGO_TARGET_DIR=target/floor cargo build --release --locked --offline
node tools/npm/release.mjs stage target/floor/release/jecode target/npm-stage/linux-x64
```

To run a build on an older distribution, unpack that distribution's root
filesystem, install its OpenSSL, and start the binary with
`unshare --user --map-root-user --mount --pid --fork` and `chroot`. No
administrator access is needed where unprivileged user namespaces are allowed.
The local Linux CI runner disables nested namespaces, so CI runs the executable
directly under the sysroot's `ld.so` instead.

From a Linux host, `sh tools/check-macos-rust.sh` lints the macOS Rust for both
architectures after `rustup target add aarch64-apple-darwin x86_64-apple-darwin`.
It needs no Apple SDK, and it does not compile the C or Objective-C adapters or
run anything: only macOS CI establishes those.

## CI

[`CI.yml`](https://github.com/giovannijecha/jecode/blob/main/.github/workflows/CI.yml)
runs formatting, the offline build, all-target Clippy and the test suite, then
the platform's native checks: fullscreen, active-history Esc, direct-input, process and LSP
panel fixtures on Windows, the PTY fixture and native conversations on Linux and macOS, and the
native TLS check on macOS. It also builds a release executable, tests npm packaging and the
install/update/uninstall lifecycle, and runs the opt-in native terminal fixture
through the npm launcher. The required checks are
`Rust (Windows)`, `Rust (Linux)`, `Rust (macOS)` and `Rust (macOS Intel)`.

Windows and Linux jobs run on maintainer-hosted runners with fresh checkouts and
one-job credentials; the host rejects fork code before workflow steps run.
These jobs queue while the maintainer's machine is offline, and a queued or
skipped job is not a successful verification. The Linux job links against the
glibc floor. macOS runs on GitHub's hosted `macos-26` ARM64 and `macos-26-intel`
runners, for native terminal checks and the `darwin-arm64` and `darwin-x64` npm
artifacts, with `MACOSX_DEPLOYMENT_TARGET=11.0`.
