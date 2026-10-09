# Development and testing

Run Cargo commands from the repository root with the pinned Rust 1.95.0 toolchain.

## Normal checks

```text
cargo build --locked --offline
cargo fmt --all -- --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline
```

For development without installation, run `cargo run --locked --offline` from this
repository. To use the development binary in another project, invoke its absolute
path. `sh INSTALL.sh` provides the corresponding Linux/macOS installation.

## Platform coverage

Tests use synthetic credentials and isolated directories under `target`, exercising
installed curl and Bash against a loopback HTTP server. They make no real OpenRouter
requests. Executed coverage: Windows `x86_64-pc-windows-gnullvm` and Linux
`x86_64-unknown-linux-musl` under WSL Ubuntu with native tmpfs fixtures, using
Rust 1.95.0. Windows checks
use curl 8.21.0 and Git Bash 5.3.9. Linux checks also cover installation, CLI
startup, process-group cleanup and the shared TUI/editor/selector/queue suites.
An isolated Linux PTY fixture exercises real terminal byte input, UTF-8 and
bracketed multiline paste, idle/active Ctrl+C, Ctrl+Q with saved input and resume,
settings cancellation, width/height changes, owner SIGKILL and failed-guardian
cleanup. It compares the original and restored terminal modes. macOS `x86_64-apple-darwin` and
`aarch64-apple-darwin` pass all-target compilation and Clippy checks; macOS linking and
execution were not run. The isolated Windows ConPTY check drives Unicode input,
wheel reports, Page Up/Down, prompt history and tiny/normal resize. Before UI
interaction it paints and reads every screen cell, including the bottom-right
corner, to detect unintended wrapping or scrolling. It checks saved
drafts, the exit summary and restoration of the original main screen, input/output
modes and output code page. Separate scenarios kill the Jecode owner and the
terminal guardian to check cleanup and recovery. These fixtures use loopback HTTP
and synthetic credentials. The shared TUI tests also cover queued prompts, menus,
tool ordering, streamed updates, `/new`, resume, exports and source reflow.
Renderer/state checks cover message distinction, composer placement, word wrapping,
Markdown fences and hanging lists, code highlighting, tool previews, capture
notices, stable tree branches, delayed tool completion, local commands, draft and
selector growth, queue recovery, exact spacing between visible blocks/panels,
stream finalization, interrupted reflow, bounded long-block layout, reading anchors,
follow-tail behavior, tiny input/selection viewports and preservation of the caller's
main screen and scrollback. Full-width screen fixtures check every column,
bottom-right painting and clearing a previously filled last cell. Styled fullscreen
previews were inspected at 80 x 24
and 40 x 12. Physical mouse-wheel/touchpad gestures and font rendering in the chosen
terminal host are separate manual checks.
Live OpenRouter authentication is outside these synthetic tests.
Development uses local Git; publishing is outside the current scope.

Navigation fixtures check continuous five-row drafts at the start, middle and
end, visual wrapping, shorter viewports, border-range updates and removal when
all text fits. Selector ranges follow navigation, filtering and empty results;
command panels reuse their title for input or suggestion ranges. Queue overflow
keeps the next message visible without a summary row, and narrow previews
prioritize text over metadata. Tool checks cover header-only preview counts,
expanded/collapsed details, retained stderr and capture-limit notices. Styled
navigation previews were inspected at 80 and 40 columns. No fixture writes the
user's configuration or contacts a real model provider.

Parallel fixture runs on WSL's shared Windows `/mnt/c` filesystem reproduced
intermittent missing-file failures; native tmpfs checks pass. A standalone
standard-library save/read probe did not reproduce them. A retained fixture
confirmed that opening could fail even while the file was present; the specific
cause remains unknown. See [PLATFORMS.md](PLATFORMS.md) for the evidence and native Mac
verification procedure, and [PERFORMANCE.md](PERFORMANCE.md) for repeatable
session and Windows startup measurements.

## Native terminal checks

The native Linux PTY fixture needs the system-provided util-linux `script` tool
for development verification; the application does not use it. Run native checks
explicitly on the matching platform:

```text
cargo test --locked --offline tui::terminal::unix::native_tests::native_linux_tui_controls_and_terminal_lifetime -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline tui::terminal::windows::native_tests::native_windows_fullscreen_controls_and_terminal_lifetime -- --exact --ignored --nocapture --test-threads=1
```

## Manual Windows console checks

The manual Windows console smoke tests use local HTTP fixtures and are ignored
by the normal test command. Run the relevant fixture in an interactive terminal:

```text
cargo test --locked --offline interactive_windows_smoke -- --ignored --nocapture
cargo test --locked --offline composer_windows_smoke -- --ignored --nocapture
cargo test --locked --offline resize_windows_smoke -- --ignored --nocapture
```

For the tool/history test, enter one task and inspect compact tool cards, then
send a second task to move
them through the owned viewport. Scroll using the wheel and Page Up/Down, browse
dispatched prompt history with Ctrl+P/N,
use `/export` and `/help`, edit a multiline draft, then `/exit`. For the composer
test, queue `/help` and another prompt during the first turn, select
`fixture/model-11` and `high` with `/model`, run `/effort low`, open/close
`/settings`, inspect a multiline draft, use `/export`, then `/exit`. The tests
use synthetic keys, validate the exported conversation and check mode restoration.
For the resize fixture, send a task, queue `/help` and a second task, resize while
waiting/streaming/running, then use `/export`, `/new`, `/settings` (Esc), `/help`
and `/exit`. Confirm the draft survives, the selector fits a short window, and
the active conversation and reading position return after a stable resize.

For a manual draft check, start a request, queue two distinct messages and stop
the turn so they become paused. Start another request and queue two more.
Open `/drafts` or press Alt+Up during work. Confirm queued rows precede paused
rows and the main composer remains intact. Enter one row, edit it, then press
Enter to save it in the same slot; repeat and use Esc to cancel an edit. Mark
one row with Ctrl+D, cancel with Esc, then mark it again and confirm with Enter.
Number keys must not confirm the discard. Press Ctrl+S on a paused draft to
send or queue only that draft. Stop a turn or resume the session and confirm
remaining unsent messages appear as separate Paused rows and do not run by
themselves. Ctrl+P/N should show dispatched prompts, never local commands;
Up/Down should move only through visual editor rows.

For conversation navigation, scroll away from the latest output and check the
centered blue Back to bottom badge above the composer. Esc returns to the bottom
while reading normally. With a panel, draft edit, history recall or tool
inspection active, Esc handles that context first; Alt+End returns to the
bottom directly. Alt+Home/End should work through panels.
