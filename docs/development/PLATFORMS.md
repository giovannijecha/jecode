# Platform support

Jecode runs natively on Windows, Linux and macOS from one codebase. Platform
differences come from the native adapters described in
[Architecture](ARCHITECTURE.md#native-boundaries). This page records what has
been verified on each platform, the limitations that remain, and how to check a
terminal host by hand.

## Verification status

| Platform | Automated verification | Interactive and live use |
| --- | --- | --- |
| Windows, x86_64 | CI on pull requests from this repository, on Windows 11: build, formatting, Clippy, the full test suite and the ConPTY terminal fixtures. Opt-in confinement, browser and light-background fixtures run locally. | Primary development platform. Live ChatGPT and Claude sign-in, inference and token refresh have been exercised. |
| Linux, x86_64 | CI on pull requests from this repository: build, formatting, Clippy, the full test suite, the PTY terminal fixture and the native conversation scenarios. Browser fixtures and X11 clipboard transfer have been run locally. | Exercised under WSL on a native Linux filesystem, including live OpenAI and Anthropic conversations with API keys on Debian 10 and Ubuntu 26.04. Wayland clipboard access is implemented but has not been exercised with a real compositor. |
| macOS, ARM64 and Intel | CI on every pull request on GitHub's hosted `macos-26` and `macos-26-intel` runners: build, formatting, Clippy, the full test suite, the PTY terminal fixture and the native conversation scenarios. | No interactive, browser or live-provider use has been recorded on a Mac. |

The Windows and Linux runners reject fork code before workflow steps. Queued,
skipped or rejected checks do not establish verification.

Other architectures, such as Windows or Linux on ARM64, have no recorded
verification. CI results apply to the revision they ran on; check the jobs for
the revision you are reviewing.

## Supported versions

Each floor is the oldest system Jecode promises to run on. Node.js 22 and 24 set
the same Linux floor and a lower Windows one, and Node.js 22 the same macOS one, so
the npm launcher cannot start below these on Linux or macOS.

| System | Floor | What sets it | Enforced by | Evidence |
| --- | --- | --- | --- | --- |
| Linux x86_64 | glibc 2.28 (Debian 10, RHEL 8, Ubuntu 20.04) | Node.js's own requirement. | CI links against a Debian 10 sysroot and runs the release under its `ld.so`. The release stage rejects any version requirement above 2.28 in the executable and its helpers; the npm launcher refuses an older glibc. | **Executed** under WSL2, on kernel 6.18, in unprivileged chroots of real images: the full test suite on Debian 10, AlmaLinux 8, Ubuntu 20.04, Ubuntu 22.04 and Debian 12, and the PTY fixture and conversation scenarios on the first three. Older kernels (Node.js asks for 4.18) have not been run. |
| Windows x64 | Windows 10 1809, build 17763 (and Server 2019) | ConPTY, for `exec` with `tty: true`. | The release stage rejects a system library outside a reviewed list, and a test rejects a new P/Invoke until it has been reviewed. | **Static only.** Imports and P/Invokes were read from the build and compared with Microsoft's documented minimums; a Windows build older than the CI host's has not run Jecode. |
| macOS, ARM64 and Intel | macOS 11.0 | Rust's own minimum on ARM, and Node.js 22's on both. Node.js 24 needs macOS 13.5. | CI and releases set `MACOSX_DEPLOYMENT_TARGET=11.0`, and the release stage rejects a higher deployment target in the executable or a helper. Adapters using an API newer than the target fail to compile. | **Not executed below the CI runner's macOS 26.** The floor comes from the recorded deployment target; no older Mac has run Jecode. |

Two Windows features depend on newer builds and turn off rather than fail. HTTP/2
needs Windows 10 1607, which is below the floor. Workspace command confinement
needs Process Security Environment support, which only recent Windows releases
have; elsewhere commands require the Full level, as on Linux kernels below 5.13.
See [Permissions](../usage/PERMISSIONS.md#platform-limits).

## Platform limitations

- **Command confinement.** Workspace confinement for `bash` and `exec` uses a
  Process Security Environment on Windows, Landlock on Linux 5.13 or later, and
  Seatbelt on macOS. Hosts without them refuse those commands until you choose the
  Full level. Seatbelt's policy has not yet run on a Mac. See
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
- **File names.** Windows compares root prefixes without case. On Linux and
  macOS, differently spelled prefixes must identify the same directory on the
  filesystem. This preserves case-sensitive volumes and lets the filesystem
  resolve case and Unicode normalization aliases. This has not run on a Mac.
- **Stopping commands.** Windows ends a command's whole job. Linux and macOS end its
  process group, so a descendant that calls `setsid` keeps running; many build
  daemons do this on purpose.
- **Connection deadline.** The native HTTP adapters apply a 10-second connection
  deadline on Windows and Linux; the macOS adapter has none.
- **Provider latency.** All three keep HTTP adapters alive between requests and
  use the OpenAI Responses WebSocket. Only Windows uses HTTP/2; Linux and macOS use
  HTTP/1.1. Reuse and the WebSocket have been measured on Windows and Linux. macOS
  has run neither, and there CFNetwork decides whether a connection is kept. See
  [Performance](PERFORMANCE.md#provider-transports).

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
| Compare `/help` and a diff with the 24-bit colours | Text is readable on every surface and added and removed rows look different. Without declared 24-bit colour, the palette's greener and redder diff rows are expected. |
| Paste multiline Unicode text, then press Ctrl+J | The text stays in the composer until you send it. Shift+Enter and Alt+Enter work only if the terminal reports them. |
| Scroll up during a response, then resize wide and narrow | The reading position stays anchored; Alt+End returns to following new output. |
| Open `/help`, `/settings` and `/routing` with a draft | Each panel keeps the draft and caret; Esc returns to the previous state. |
| Queue a message, interrupt, quit and resume | Drafts and pending messages are restored without an interrupted command running again. |
| Delete a disposable conversation from `/resume` | Ctrl+D marks it, Enter confirms, the list stays open and the content stays gone after a resize. |
| Quit with Ctrl+Q | The previous shell screen and terminal modes are restored. |
| `/copy` an answer with Unicode, trailing spaces, a code block and a quote | Each pasted selection matches the source exactly. |
| Drop a file whose path contains spaces, and paste an image with Alt+V or, on Linux and macOS, Ctrl+V | Both attach; after resume and `/export`, the export references the attachment without copying it. |
| On a Mac, type `@`, `[` and `{` with Option on your keyboard layout | The characters appear in the draft; none triggers a shortcut. |
