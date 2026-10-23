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
| macOS, Intel and Apple Silicon | All-target cross compilation and Clippy. | Native linking, test execution and terminal interaction on a Mac. |

These are local verification boundaries. CI jobs and ignored/manual fixtures
provide separate evidence; inspect the checks executed for the revision under review.

The backend separation was checked locally on October 23, 2026: formatting,
offline build and Clippy passed with 943 ordinary tests on Windows and 938 on
Ubuntu under WSL using a fresh native-filesystem copy. Both installed-browser
fixtures and the platform's native terminal fixture passed on each host. The
ordinary suite includes different-protocol backend switching, save failure,
legacy origin retention, account credential masking and expired endpoint refresh. This change has no
new native macOS execution evidence.

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

Requires Windows 10+, Git Bash, Windows PowerShell/.NET and ConPTY:

```text
cargo test --locked --offline tui::terminal::windows::native_tests::native_windows_fullscreen_controls_and_terminal_lifetime -- --exact --ignored --nocapture --test-threads=1
```

The fixture checks a full-width console grid, Unicode and dropped-path input,
wheel/page navigation, prompt history, endpoint selection and resize. It then
checks saved drafts and restoration of the shell screen, console modes and output
code page. Separate scenarios terminate the owner and terminal guardian.

Checkpoint preimages use standard-library file I/O and operating-system file locks. The advisory disk-space probe uses the owned cached PowerShell/.NET helper.

## Linux and WSL

Requires the system util-linux `script` tool in addition to the runtime dependencies:

```text
cargo test --locked --offline tui::terminal::unix::native_tests::native_linux_tui_controls_and_terminal_lifetime -- --exact --ignored --nocapture --test-threads=1
```

The fixture drives Unicode paste, queued messages, selectors, scrolling,
cancellation, resize, resume and deletion through a PTY. It checks terminal modes
after normal exit, owner SIGKILL and guardian loss.

Use a repository copy on a native Linux filesystem for the full suite. Parallel
tests under WSL's Windows-backed `/mnt/c` have intermittently failed to open files
that remained present. Native tmpfs runs passed; standalone save/read probes did
not isolate the cause. Treat a `/mnt/c` failure as unresolved evidence and retain
its fixture rather than masking it with retries.

Checkpoint capture has no notification helper or background command scan on any platform. Unix admission uses the system df command. Platform CI runs the ordinary suite and its native terminal fixture for the checked-out revision.

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
