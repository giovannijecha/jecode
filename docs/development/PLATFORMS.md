# Native platform verification

Jecode shares its agent loop and multiline TUI across Windows, Linux and macOS.
Compilation is separate from native execution coverage. macOS runtime checks
remain pending until a Mac is available. Run the commands below from the
repository root unless a section states otherwise.

## Fullscreen verification

After the 2026-10-03 full-width and compact-navigation updates, the offline build,
formatting, Clippy and test gates passed on Windows (398 passed, 10 ignored) and
Linux with native tmpfs fixtures (397 passed, 8 ignored). Both explicit native
terminal checks passed.
Intel and Apple Silicon macOS passed all-target compilation and Clippy; linking
and native macOS execution remain pending. No third-party dependencies were added.

The owned VT screen model checks alternate/main buffer separation, restored shell
content and cursor, fixed-bottom input, changed-row painting and absence of saved-line
erasure. Full-width fixtures fill every cell, including the bottom-right corner,
check that no rows scroll and clear a previously filled last cell on redraw.
Navigation fixtures check continuous composer windows, muted ranges in borders
and headings, selector filtering, overflow without extra summary rows and tool
expansion without stale preview metadata. Styled navigation previews were
inspected at 80 and 40 columns.
Shared fixtures cover wheel/page scrolling without draft/history changes,
follow-tail behavior, reading during streaming and panels, source anchors across
reflow, multiline table headers, active-conversation reset and bounded layout work.
Styled fullscreen previews were inspected at 80 x 24 and 40 x 12 for welcome,
conversation/tool cards, reading, settings, commands and a queued multiline draft.

The isolated Windows ConPTY fixture first compares a full-width native console
snapshot with the painted grid, including its bottom-right cell. Its console
probe reads a counted character buffer. It then drives Unicode prompts/drafts, SGR reports
converted by the console into native wheel records, Page Up/Down, Up/Down history,
tiny/normal resize and Ctrl+Q. It checks saved drafts, the two-line exit summary,
the previous shell screen, exact input/output modes and the output code page.
Separate scenarios kill the Jecode owner and its guardian. The Unix PTY fixture
drives wheel/page input alongside its existing paste, queue, menu, cancellation,
resume and deletion checks; it checks balanced alternate-screen entry/exit and
restored terminal modes after normal exit, owner SIGKILL and forced guardian loss.
Expected owner-pipe EOF is a successful guardian shutdown; failed guardians use
the owner's recovery path.

Run the matching native check explicitly:

```text
cargo test --locked --offline tui::terminal::windows::native_tests::native_windows_fullscreen_controls_and_terminal_lifetime -- --exact --ignored --nocapture --test-threads=1
cargo test --locked --offline tui::terminal::unix::native_tests::native_linux_tui_controls_and_terminal_lifetime -- --exact --ignored --nocapture --test-threads=1
```

Windows verification uses the system ConPTY APIs and PowerShell/.NET; Unix uses
the system `script` utility. These are development fixtures, with loopback HTTP,
synthetic credentials and isolated files. Physical wheel/touchpad gestures, host
key interception and font rendering in Windows Terminal or another chosen host
remain manual checks. Mouse selection is deliberately omitted; `/copy` remains
the supported way to copy original completed content. See [RESIZE.md](../tui/RESIZE.md).

## macOS

Use a repository copy on the Mac's native filesystem, Rust 1.95.0 and Apple's
Command Line Tools. The system Bash, HTTPS-enabled curl, kill, stty, mkfifo, rm
and rmdir must be available. Verification also uses the system tty and script
utilities; these two tools are not application dependencies.
`/copy` also uses the system `/usr/bin/pbcopy` command.

Run the normal gates first:

```text
cargo build --locked --offline
cargo fmt --all -- --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline
```

The ordinary process tests include loss of Jecode without Rust destructors,
SIGKILL, cancellation after the original shell exits, input EOF and large binary
input. The ignored native PTY fixture is adapted to macOS's system
[script invocation](https://github.com/apple-oss-distributions/shell_cmds/blob/main/script/script.1)
and is compiled by the cross checks, but has not been executed on macOS:

```text
cargo test --locked --offline tui::terminal::unix::native_tests::native_macos_tui_controls_and_terminal_lifetime -- --exact --ignored --nocapture --test-threads=1
```

It uses loopback HTTP, synthetic credentials and isolated files under `target`.
It drives multiline Unicode paste, queueing, selectors, tiny/normal dimensions,
idle and active Ctrl+C, Ctrl+Q, resume, owner SIGKILL and loss of the terminal
guardian. It compares saved and restored terminal modes and checks that a
cancelled command makes no late write. Do not run the private `native_pty_fixture`
test directly.

Finally run the installed TUI in Terminal or another VT-compatible host with
the maintainer's normal configuration. Check actual key reporting, mouse-wheel
conversation scrolling, paste and redraw while resizing. Ctrl+J is the portable newline;
Shift/Alt+Enter depend on the host. Ctrl+Q exits and Ctrl+C stops work or clears
input. Record the macOS version, architecture and terminal host before claiming
native coverage. Live authentication is a separate manual check.

Use `/copy` after a completed answer containing Unicode, trailing spaces,
multiple fenced blocks and an explicit quote. Paste each choice into an editor
and compare it with the original source, including line endings. Repeat while
another turn streams and confirm Esc/Ctrl+C only closes the copy selector. Test
after `/resume`, and check that `/new` has no previous response to copy. Plain
mode has a numbered selector. These manual copy checks deliberately write the
clipboard; the ordinary synthetic tests never access it. See [COPY.md](../usage/COPY.md)
for the OSC 52 fallback for literal EPS/RTF signatures.

## Copy verification

After adding `/copy`, the 2026-10-01 Windows offline build, formatting, Clippy
and test gates passed: 288 tests passed and 8 were ignored. Linux passed the same
gates with native tmpfs fixtures: 284 tests passed and 8 were ignored, plus the
explicit native PTY check passed. Intel and Apple Silicon macOS passed all-target
compilation and Clippy checks; native execution remains pending.

Copy-specific fixtures cover stable selection during streaming, preserved
Unicode/CRLF/trailing spaces, searchable and tiny menus, Esc/Ctrl+C without turn
cancellation, resume/new boundaries, visible masked failures and plain-mode
cancellation without another provider request. Clipboard fixtures verify the
Windows script's exact UTF-8 input and bounded OSC 52 output without touching
the user's clipboard. Native `Set-Clipboard` and host acceptance of OSC 52 have
not been verified. A separate Windows probe refused clipboard operations because
it could not create a new private window station; the unnamed create-only attempt
returned Win32 error 183 (already exists). This does not indicate an application
clipboard failure. Microsoft documents each [window station's separate clipboard](https://learn.microsoft.com/en-us/windows/win32/winstation/about-window-stations-and-desktops).

## Markdown verification

After extending response rendering, the 2026-10-01 offline build, formatting,
Clippy and test gates passed on Windows (310 passed, 8 ignored) and Linux with
native tmpfs fixtures (306 passed, 8 ignored). The explicit Linux native PTY
check also passed. Intel and Apple Silicon macOS passed all-target compilation
and Clippy checks; native macOS execution remains pending.

Markdown fixtures cover table alignment, wrapped cells and vertical fields,
partial streaming, finalization, resize reconstruction, source preservation,
inline styles, code whitespace and common Unicode display groups. A layout
preview generated from the renderer's styled rows was inspected at 80, 40 and
18 columns. VT screen fixtures check that provisional tables never enter saved
history and that the final layout appears once with draft and queue preserved.
These checks do not establish identical glyph rendering across native fonts.
On a native terminal, compare an accented/emoji table at wide and narrow widths,
resize while it streams, and confirm `/copy` retains its original Markdown.
See [MARKDOWN.md](../tui/MARKDOWN.md) for the supported display subset.

## Session and menu verification

After session-management, menu-panel and transient-feedback updates, the 2026-10-02 offline build,
formatting, Clippy and test gates passed on Windows and Linux with native tmpfs
fixtures (383 passed and 8 ignored on each platform). The explicit Linux native
PTY check also passed.
Intel and Apple Silicon macOS passed all-target compilation and Clippy checks;
native macOS execution remains pending. No third-party dependencies were added.

Command suggestions, arguments and selectors share a borderless shaded panel.
Owned styled previews were inspected for commands, settings, search, masked key
input, deletion, empty lists and errors at wide/narrow widths. Regression checks
cover panel contrast and caret backgrounds, compact keyboard hints, active
command controls, preserved draft/cursor, very short/tiny viewports and erasure
without saving menus to native history. The native Linux PTY fixture checks the
separate key form before cancellation and retains the session-management checks.

Feedback fixtures cover five-second expiry, dismissal by editing, errors retained
until user action, independent copy results and silent menu cancellation.
They exercise real failed/recovered journal writes and unread errors during
worker completion, connection/maintenance events and automatic queue dispatch,
including model loading and a new conversation. Help and temporary-file panels
retain draft/caret, support visual-row/page scrolling and close without sending
the draft. Styled previews were inspected at 100 and 40 columns; owned VT screen
checks show that dismissed feedback/panels and archived UI records stay absent
from native history and resize reconstruction. Resume retains conversation and
audit records without replaying old UI messages. Loading uses one operation title.

Shared fixtures cover first-request saving, cancellation, active/inactive deletion,
exclusive leases, retired autosave handles, current context/screen reset, preserved
draft/queue input and the next request's clean context. Owned output and temporary
trees, legacy references, recovery files, rejected links/junctions, partial
failures and preserved project/export files have focused checks. Regression checks
cover restoration of output/temp ownership after a failed final directory removal,
retry with the journal intact, and the original draft/cursor while browsing history.
Fixtures also exercise Ctrl+D/Enter confirmation in the same `/resume` list,
Esc/Ctrl+C cancellation, search/navigation disarming, filtered row identity,
numeric shortcut isolation, repeated deletion, worker input freezing, errors
and saving the deletion result before exit. Empty launches and unsent drafts do
not count as current sessions. Current and last-session deletion keep `/resume`
open, pause the queue until leaving it, and show an empty-list message when needed.
Settings checks cover repeated default changes, updated values, visible feedback,
key success/rejection, catalogue errors, cancelled child/loading menus and closing
the manager explicitly. Standalone model/effort selection still ends normally.
A preview of the renderer's styled
list rows was inspected at 80, 40 and 18 columns and in short viewports, including
pending confirmation, loading and errors. The native Linux PTY check sends the
real Ctrl+D bytes to cancel, delete a saved row while retaining the list, and
delete the current/last row while retaining the empty list over a fresh context.
It also checks Ctrl+C from key input returns to settings before closing that menu.
VT screen checks ensure old
visible/native history does not return after resize.

One earlier parallel Windows run captured Git Bash's startup warning about a
missing `/tmp` in three shell-output fixtures. Their isolated reruns and the
final unmodified normal gate invocation passed; its cause remains unisolated.
These transient failures were not addressed by changing application behavior.

On a Mac, use a disposable conversation to check `/resume`, Ctrl+D on a row,
Esc/Ctrl+C to cancel the mark, Enter to confirm and the fresh current screen.
Check that its unsent draft/queue survives, the list stays open, an empty
replacement stays absent until a request is sent, and the old conversation stays
absent after resizing and restarting. Check settings child cancellation and
repeated changes without returning to the composer. Terminal glyphs
and restoration of the shell screen still need verification in the chosen host.
Open `/help` with a multiline draft, page to the end in a small viewport, then
close with Esc and by typing; confirm the draft/caret is preserved. Check that
short feedback expires after five seconds, errors wait for the next action and
dismissed UI rows stay absent after resizing and resume.

## Attachment verification

On Windows 11 with Windows PowerShell 5.1, the ConPTY fixture pastes a quoted
path the way Windows Terminal delivers a drop and checks the saved draft holds
the element beside typed Unicode text. The conversion check turned a synthetic
3000x10 BMP into a 2048x7 PNG, and the clipboard check read a real PNG image
without printing its contents. Drop, Alt+V, `/attach`, queueing, recall,
resume, export and provider parts are covered by isolated fixtures.

Through Webterminal, a headless Chrome probe dropped a binary from outside the
working directory onto an isolated debug Jecode. Jecode attached it with
identical bytes, and Alt+V attached the clipboard image.

Under WSL (Ubuntu, WSL2), `wslpath` and `powershell.exe` standard-input interop
were probed directly; Jecode itself was not built inside WSL, and the Linux and
macOS capture paths were not compiled. A physical mouse drop into Windows
Terminal or a browser, and macOS hosts, remain manual checks.

## Linux and WSL

Run the same normal gates on Linux. The equivalent ignored PTY check uses the
system util-linux script utility:

```text
cargo test --locked --offline tui::terminal::unix::native_tests::native_linux_tui_controls_and_terminal_lifetime -- --exact --ignored --nocapture --test-threads=1
```

WSL checks should distinguish a native Linux filesystem from Windows-backed
`/mnt/c`. The full parallel suite has reproduced intermittent missing-file
errors on `/mnt/c`, including successful saves followed by immediate failed
reads in configuration and session tests. A retained file fixture was present
in the directory listing during failure: path canonicalization succeeded, then
opening the file returned ENOENT. Windows and WSL subsequently read the same
20-byte file. This supports a transient lookup/open inconsistency, rather than
fixture deletion, without identifying its cause. Standalone standard-library probes
passed 42,000 Windows-backed and 12,800 tmpfs operations with synced rename,
negative pre-save lookups and parallelism up to 64 threads. The interacting
condition is still unknown: this evidence does not establish a general 9p bug
or justify retries that mask the failures. The verification boundary must stay
visible until the cause is isolated.
