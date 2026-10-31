# Native platform verification

Run Cargo commands from the repository root. Start with the
[normal checks](TESTING.md#normal-checks), then run the native fixture for the
host you are testing. Fixtures use loopback HTTP, synthetic credentials and
isolated files.

## Local coverage

| Platform | Recorded local verification | Remaining host checks |
| --- | --- | --- |
| Windows, `x86_64-pc-windows-gnullvm` | Offline build, formatting, Clippy, ordinary tests and the ConPTY fixture. | Physical input, fonts and clipboard behavior in the chosen terminal. |
| Linux under WSL, native tmpfs or ext4 | Offline build, formatting, Clippy, ordinary tests and the Linux PTY fixture. | Physical terminal integration; Windows-backed `/mnt/c` has an unresolved fixture issue below. |
| macOS, Intel and Apple Silicon | Native adapter sources implemented; this runtime change has no executed Mac build. | Native compilation, linking, tests and terminal interaction on a Mac. |

These are local verification boundaries. CI jobs and ignored/manual fixtures
provide separate evidence; inspect the checks executed for the revision under review.

The October 31 Claude subscription and context-efficiency changes passed offline
build, formatting, all-target Clippy and ordinary tests on Windows (1,867 passed,
68 ignored) and an Ubuntu/WSL native-filesystem copy (1,867 passed, 56 ignored). The Windows
ConPTY fixture passed with the new credentials, Claude account and sign-in
panels; the Linux PTY fixture also passed. One Windows run concurrent with the
ordinary suites exceeded its 20-second wait for the preexisting card fixture;
the isolated rerun passed without a code change. Run terminal fixtures after
the ordinary suite. Live Claude login, tool inference, rotating refresh and
conversation resume were verified on Windows with an isolated home. This pass
has no live Anthropic or native UI evidence on macOS.
Bounded account checks also verified growing-history cache reuse on Claude and
ChatGPT and a medium-effort summary followed by resumed high-effort fact
readback; see [performance](PERFORMANCE.md#subscription-context-reuse).
The Windows IPv6 HTTP fixture now explicitly restores blocking mode after
accepting a socket from its nonblocking listener; its regression passed 30
consecutive runs after the CI `WouldBlock` failure was traced to that mode.
Windows record fixtures also preserve a control-character shortcut when its
matching letter key has no Ctrl modifier flag, including during a text burst.
The startup ConPTY check verifies the composer is empty after clearing a
retained request before sending the next command.

The October 29 native-runtime pass completed the ordinary suite on Windows
(1,721 passed, 57 ignored) and an Ubuntu/WSL native-filesystem copy (1,725 passed,
49 ignored). Windows build, formatting and all-target Clippy passed; Linux
all-target Clippy passed. The fullscreen ConPTY, permissions and light-background
fixtures passed separately. The light check runs the real TUI against an
isolated white console palette and verifies the native fallback after an
unanswered OSC 11 query; it does not establish OSC replies through ConPTY.
The Linux PTY fixture also passed, including interruption, session recovery,
owner death and terminal-mode restoration.
Both platforms built release packages and exercised install/update plus the
owned interpreter with an empty PATH. Controlled X11 clipboard transfer and
Linux image conversion were checked separately; Wayland data-control remains
protocol-checked but not exercised with an isolated compositor. No Mac was run.

The backend separation was checked locally on October 23, 2026: formatting,
offline build and Clippy passed with 943 ordinary tests on Windows and 938 on
Ubuntu under WSL using a fresh native-filesystem copy. Both installed-browser
fixtures and the platform's native terminal fixture passed on each host. The
ordinary suite includes different-protocol backend switching, save failure,
legacy origin retention, account credential masking and expired endpoint refresh. This change has no
new native macOS execution evidence.

The request-transport correction was checked on October 23, 2026 with curl
8.21.0 on Windows and 8.18.0 on Ubuntu under WSL. Formatting, offline build,
Clippy and the ordinary suite passed: 956 Windows tests and 950 Linux tests.
Both native browser fixtures passed on each host, including three consecutive
Windows runs, along with ConPTY and Linux PTY checks. Loopback regressions cover
requests above 10 MiB, literal `@` data, exact image bytes, a native write followed
by `avanti`, incomplete stdin diagnostics and Windows child environment overrides.

## Collaborator workspace boundary

Shared collaborators use the current project. Isolated assignments copy the
current project files to Jecode-owned storage without requiring Git or `HEAD`.
The snapshot excludes `.git` and `.jecode` entries at every level, plus root
directories named `target` or `.tmp`. Other files and empty directories are
included, even when Git ignores them. Review and
selected-path integration compare the copy, starting state and current project.
The October 28 Windows implementation pass covered dirty baselines,
concurrent-edit conflicts, missing and foreign ownership markers,
and interrupted setup recovery. New collaborator behavior has no native Linux
or macOS execution evidence from the October 28, 2026 implementation pass.

That Windows pass completed offline build, formatting, all-target Clippy and
1533 ordinary tests (44 ignored), plus the separately executed fullscreen ConPTY
fixture. Its collaboration checks paint receipts, `/agents`, footer badge and
gauge through the native renderer. A real configured-model run also completed
two assignments, an in-flight revision, selected-path integration and review.

On October 29, 2026, the owned-copy implementation passed 16 focused workspace
tests and 11 collaboration tests on Windows, including preparation, integration
and cleanup in a folder without `.git` and with Git absent from `PATH`.

## Interactive command boundary

`exec` launches literal executable arguments through the existing process
supervisor. With `tty:true`, an owned native bridge supplies a pseudo-terminal:
Windows uses ConPTY and current-user-only named pipes, with the opened server
handle checked against the launched bridge PID before any input is sent.
Linux and macOS use POSIX PTYs and private Unix-domain sockets. Input and control
have separate connections, so bridge input backpressure cannot block resize
or delivery of an interrupt request. Input travels over its dedicated
connection and is not staged in regular files. Terminal output combines stdout
and stderr and remains available through the normal saved-output references.

Unix interrupts pass through the terminal line discipline. The configured
`VINTR` character produces signals only while `ISIG` is enabled; otherwise the
application receives literal ETX input. `NOFLSH` preserves pending input. When
flushing is enabled, input generations prevent older bytes still in the Rust
writer or socket from being replayed after the interrupt, while preserving later input.
Without a foreground process group, including after a client detaches its
controlling terminal, an interrupt is consumed as a no-op without flushing input.
Only a temporarily full master keeps the request queued. Raw ETX intentionally
has priority over data still buffered in the bridge.

Windows builds the owned adapters with its system .NET compiler and uses Windows
10+ ConPTY at runtime. Unix builds the owned bridges with system `cc` during the
Cargo build; runtime uses the embedded binaries without a compiler. `CC`
overrides it. Cross builds need `CC_<target>` or `TARGET_CC`, except between
macOS architectures, where `cc -arch` is used. Linux
enumerates `/proc` to clean up interactive job-control groups;
macOS uses its system `/bin/ps`. Debug builds deny C compiler warnings;
release builds report them without failing.
Control sockets need a native Unix temporary filesystem; WSL's Windows-backed
mounts do not support them. Local input bypasses model messages and tool-call
history, while output that the target echoes remains visible and retained.
The fullscreen terminal panel displays a transcript rather than emulating
cursor-addressed full-screen applications.

## Local web and browser tools

The web HTTP, crawl recovery and WebSocket fixtures run in the ordinary suite.
The installed-browser checks are opt-in and use loopback pages, synthetic state
and dedicated profiles without a model API request or an automation package:

```text
cargo test --locked --offline agent::browser_tests::native_browser_tool_delivers_images_and_preserves_only_named_profile_state -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline browser::tests::live_chromium_localhost_interaction_and_profile_persistence -- --exact --ignored --nocapture --test-threads=1
```

On October 23, 2026, the browser checks passed on Windows with Chrome
155.0.8059.86 and Ubuntu under WSL on a native Linux filesystem with Chrome
155.0.8059.79. They exercise navigation, offscreen reference clicks, form input,
screenshots and model attachment delivery, persistent local state, interrupted
evaluation recovery, temporary cleanup and live-session deletion. The Windows
ConPTY and Linux PTY fixtures also passed; the Windows renderer fixture includes
the Web and Browser cards. Native browser execution on macOS remains unverified.

## Windows

Requires Windows 10+, Windows PowerShell/.NET and ConPTY:

```text
cargo test --locked --offline tui::terminal::windows::native_tests::native_windows_fullscreen_controls_and_terminal_lifetime -- --exact --ignored --nocapture --test-threads=1
```

The fixture checks a full-width console grid, Unicode and dropped-path input,
wheel/page navigation, prompt history, draft preservation, endpoint selection
and resize. It then checks restoration of the shell screen, console modes and output
code page. Separate scenarios terminate the owner and terminal guardian.

Checkpoint preimages use standard-library file I/O and operating-system file
locks. The advisory disk-space probe uses the embedded .NET API adapter.

## Linux and WSL

Requires the system util-linux `script` tool in addition to the runtime dependencies:

```text
cargo test --locked --offline tui::terminal::unix::native_tests::native_linux_tui_controls_and_terminal_lifetime -- --exact --ignored --nocapture --test-threads=1
```

The fixture drives Unicode paste, queued messages, selectors, scrolling,
cancellation, resize, resume and deletion through a PTY. It checks terminal modes
after normal exit, owner SIGKILL and guardian loss.

On October 29, 2026, the embedded Linux clipboard and image adapter compiled
under WSL with `cc -Wall -Wextra -Werror`. Synthetic BMP images converted to
bounded PNG and smaller JPEG output. An isolated Xvfb server and owned test
selection source transferred PNG bytes through X11 without reading the real
clipboard. No-display and invalid-image failures were checked. This does not
establish Wayland compositor behavior or a full native Linux Rust test run;
Wayland data-control access depends on the compositor advertising the protocol.

Use a repository copy on a native Linux filesystem for the full suite. Parallel
tests under WSL's Windows-backed `/mnt/c` have intermittently failed to open files
that remained present. Native tmpfs runs passed; standalone save/read probes did
not isolate the cause. Treat a `/mnt/c` failure as unresolved evidence and retain
its fixture rather than masking it with retries.

Checkpoint capture has no notification helper or background command scan on any
platform. Unix admission reads filesystem capacity through the embedded native
adapter's `statvfs` call. Platform CI runs the ordinary suite and its native
terminal fixture for the checked-out revision.

## macOS

Use a native filesystem, Rust 1.95.0 and Apple's Command Line Tools. The native
fixture also needs the system `script` and `tty` utilities:

```text
cargo test --locked --offline tui::terminal::unix::native_tests::native_macos_tui_controls_and_terminal_lifetime -- --exact --ignored --nocapture --test-threads=1
```

This fixture uses macOS's `script` invocation and exercises the Unix control and
lifetime scenarios above. The hosted macOS CI job invokes it; inspect the job
result for evidence of native execution. Do not invoke its private
`native_pty_fixture` child directly.

## Check the terminal host

After automated checks, use a disposable conversation in the terminal you support.
Record the OS version, architecture and terminal host with the result.

| Check | Expected result |
| --- | --- |
| Paste multiline Unicode text; use Ctrl+J | Text stays in the composer until submitted. Host-specific Shift/Alt+Enter mappings are optional. |
| Scroll during a response, then resize wide/narrow | Reading position stays anchored; returning to the bottom follows new output. |
| Open `/help`, `/settings` and `/routing` with a draft | Panels preserve the draft and cursor; cancellation returns to the preceding state. |
| Queue messages, interrupt, quit and resume | Saved drafts/pending messages remain available without silently repeating an interrupted command. |
| Delete a disposable session through `/resume` | Ctrl+D marks, Enter confirms, the list stays open and old content stays absent after resize. |
| Quit with Ctrl+Q | The preceding shell screen and terminal modes are restored. |

Physical wheel/touchpad gestures, key interception, emoji/font rendering and
clipboard delivery require the real host. Synthetic fixtures do not establish
those results.

### Copy and attachments

Use `/copy` on an answer with Unicode, trailing spaces, fenced code and a quote.
Paste each selection into an editor and compare it with the source. Repeat after
resume and while another turn streams; Esc/Ctrl+C should close the copy selector.
This deliberately writes the clipboard. Native delivery requirements and fallbacks
are in [copy](../usage/COPY.md).

Drop a file with spaces in its path and import an image with Alt+V. Wait for
importing, submit, then resume and export the conversation. Confirm the owned
attachment survives and the export contains its reference in the JSON without
copying the file. See
[attachments](../usage/ATTACHMENTS.md) for platform-specific capture requirements.
