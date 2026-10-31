# Platform support

Jecode runs natively on Windows, Linux and macOS from one codebase. Platform
differences come from the native adapters described in
[Architecture](ARCHITECTURE.md#native-boundaries). This page records what has
been verified on each platform, the limitations that remain, and how to check a
terminal host by hand.

## Verification status

| Platform | Automated verification | Interactive and live use |
| --- | --- | --- |
| Windows 10 or later, x86_64 | CI on pull requests from this repository: build, formatting, Clippy, the full test suite and the ConPTY terminal fixtures. Opt-in confinement, browser and light-background fixtures run locally. | Primary development platform. Live ChatGPT and Claude sign-in, inference and token refresh have been exercised. |
| Linux, x86_64 | CI on pull requests from this repository: build, formatting, Clippy, the full test suite and the PTY terminal fixture. Browser fixtures and X11 clipboard transfer have been run locally. | Exercised under WSL on a native Linux filesystem. Wayland clipboard access is implemented but has not been exercised with a real compositor. |
| macOS | CI on every pull request on GitHub's hosted `macos-15` runner: build, formatting, Clippy, the full test suite and the PTY terminal fixture. | No interactive, browser or live-provider use has been recorded on a Mac. |

The Windows and Linux runners reject fork code before workflow steps. Queued,
skipped or rejected checks do not establish verification.

Other architectures, such as Windows or Linux on ARM64, have no recorded
verification. CI results apply to the revision they ran on; check the jobs for
the revision you are reviewing.

## Platform limitations

- **Command confinement.** Workspace confinement for `bash` and `exec` uses a
  Windows-only OS facility. On Linux and macOS, and on Windows hosts without it,
  commands require the Full level. See
  [Permissions](../usage/PERMISSIONS.md#platform-limits).
- **Interactive terminals.** Interactive `exec` (`tty: true`) requires Full on
  every platform. The `/terminal` view shows a transcript of the output; it does
  not emulate full-screen terminal applications.
- **Unix control sockets.** Interactive terminals need a native Unix temporary
  filesystem. Under WSL, Windows-backed mounts such as `/mnt/c` do not support
  them.
- **WSL test runs.** Run the test suite from a copy on a native Linux filesystem.
  Parallel tests on `/mnt/c` have intermittently failed to open files that were
  present; the cause is not established.
- **Clipboard.** Copying uses PowerShell on Windows, `pbcopy` on macOS and the
  terminal's OSC 52 support on Linux. See [Copy and export](../usage/COPY_AND_EXPORT.md#platform-requirements).
- **Image attachments.** Clipboard images and image conversion depend on
  PowerShell (Windows and WSL), `osascript` and `sips` (macOS), or X11 or
  Wayland data control and GdkPixbuf (Linux). See
  [Attachments](../usage/ATTACHMENTS.md#platform-requirements).
- **Connection deadline.** The native HTTP adapters apply a 10-second connection
  deadline on Windows and Linux; the macOS adapter has none.

## Interactive terminal bridge

`exec` with `tty: true` starts an owned native bridge. On Windows it uses ConPTY
and current-user named pipes, and checks the opened pipe against the launched
bridge's process ID before sending input. On Linux and macOS it uses a POSIX PTY
and private Unix-domain sockets. Input and control use separate connections, so
input backpressure cannot block a resize or an interrupt. Terminal output
combines stdout and stderr and is saved like other command output.

On Unix, interrupts pass through the terminal line discipline. The `VINTR`
character produces a signal only while `ISIG` is enabled; otherwise the
application reads a literal ETX byte. `NOFLSH` preserves pending input; when
flushing is on, older input still in transit is discarded while later input is
kept. Without a foreground process group, an interrupt is a no-op.

The bridges are built during the Cargo build: with the system .NET compiler on
Windows, and with `cc` on Linux and macOS (override with `CC`; cross builds need
`CC_<target>` or `TARGET_CC`, except between macOS architectures, which use
`cc -arch`). Debug builds treat C compiler warnings as errors. Linux cleans up
interactive job-control groups through `/proc`; macOS uses `/bin/ps`.

## Native fixtures

The terminal, confinement, browser and live-provider fixtures are listed in
[Development and testing](TESTING.md#native-terminal-fixtures). Each uses
loopback HTTP, synthetic credentials and isolated files.

## Check a terminal host

Synthetic fixtures cannot establish physical input, key interception, font and
emoji rendering or clipboard delivery. After the automated checks, use a
disposable conversation (for example with `JECODE_HOME` set to an absolute
scratch directory) in the terminal you want to support, and record the OS
version, architecture and terminal with the result.

| Check | Expected result |
| --- | --- |
| Paste multiline Unicode text, then press Ctrl+J | The text stays in the composer until you send it. Shift+Enter and Alt+Enter work only if the terminal reports them. |
| Scroll up during a response, then resize wide and narrow | The reading position stays anchored; Alt+End returns to following new output. |
| Open `/help`, `/settings` and `/routing` with a draft | Each panel keeps the draft and caret; Esc returns to the previous state. |
| Queue a message, interrupt, quit and resume | Drafts and pending messages are restored without an interrupted command running again. |
| Delete a disposable conversation from `/resume` | Ctrl+D marks it, Enter confirms, the list stays open and the content stays gone after a resize. |
| Quit with Ctrl+Q | The previous shell screen and terminal modes are restored. |
| `/copy` an answer with Unicode, trailing spaces, a code block and a quote | Each pasted selection matches the source exactly. |
| Drop a file whose path contains spaces, and paste an image with Alt+V | Both attach; after resume and `/export`, the export references the attachment without copying it. |
