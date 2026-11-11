# Platform support

Jecode runs natively on Windows, Linux and macOS from one codebase. Platform
differences come from the native adapters described in
[Architecture](../development/ARCHITECTURE.md#native-boundaries). This page lists
system requirements, verification status and remaining platform limitations.

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

One Windows feature depends on newer builds and turns off rather than fails:
Workspace command confinement needs Process Security Environment support, which only
recent Windows releases have; elsewhere commands require the Full level, as on Linux
kernels below 5.13. HTTP/2 needs Windows 10 1607, below the floor, so every supported
system has it.
See [Permissions](PERMISSIONS.md#platform-limits).

### Runtime and feature requirements

| Platform | Required | Needed only for some features |
| --- | --- | --- |
| Windows | The native console. | Windows PowerShell 5.1 for `/copy` and clipboard images (Alt+V). |
| Linux | A VT-compatible terminal. The system OpenSSL libraries (`libssl.so.3` and `libcrypto.so.3`, or the 1.1 versions) for HTTPS and for hashing session and permission data. Minimal container images often lack them. | `libX11.so.6` or `libwayland-client.so.0` for clipboard images, GdkPixbuf for image conversion, and a terminal that accepts OSC 52 for `/copy`. |
| macOS | A VT-compatible terminal. | None beyond the system tools. |

The npm launcher also needs Node.js 22 or later. On macOS 11 to 13.4, use
Node.js 22; Node.js 24 requires macOS 13.5 or later. A standalone executable
does not need Node.js.

## Platform limitations

- **Command confinement.** Workspace confinement for `bash` and `exec` uses a
  Process Security Environment on Windows, Landlock on Linux 5.13 or later, and
  Seatbelt on macOS. Hosts without them refuse those commands until you choose the
  Full level. Seatbelt's policy is exercised by macOS CI; macOS 11 remains
  unverified. See
  [Permissions](PERMISSIONS.md#platform-limits).
- **Interactive terminals.** Interactive `exec` (`tty: true`) requires Full on
  every platform. The `/process` view shows a transcript of the output; it does
  not emulate full-screen terminal applications.
- **Unix control sockets.** Interactive terminals need a native Unix temporary
  filesystem. Under WSL, Windows-backed mounts such as `/mnt/c` do not support
  them.
- **Clipboard.** Copying uses PowerShell on Windows, `pbcopy` on macOS and the
  terminal's OSC 52 support on Linux. See [Copy and export](COPY_AND_EXPORT.md#platform-requirements).
- **Image attachments.** Clipboard images and image conversion depend on
  PowerShell (Windows and WSL), `osascript` and `sips` (macOS), or X11 or
  Wayland data control and GdkPixbuf (Linux). See
  [Attachments](ATTACHMENTS.md#platform-requirements).
- **File names.** Windows compares root prefixes without case. On Linux and
  macOS, differently spelled prefixes must identify the same directory on the
  filesystem. This preserves case-sensitive volumes and lets the filesystem
  resolve case and Unicode normalization aliases. Case-sensitive APFS volumes
  remain unverified.
- **Stopping commands.** Windows ends a command's whole job. Linux and macOS end its
  process group, so a descendant that calls `setsid` keeps running; many build
  daemons do this on purpose.
- **Connection deadline.** The native HTTP adapters apply a 10-second connection
  deadline; on Unix it bounds each address attempt.
- **Provider latency.** All three keep HTTP adapters alive between requests and
  use the OpenAI Responses WebSocket. Only Windows uses HTTP/2; Linux and macOS use
  HTTP/1.1. Provider latency has been measured only on Windows; no comparable Linux
  or macOS timing is recorded. Linux and macOS share the owned
  HTTP/1.1 parser, connection cache and replay policy, with OS TLS underneath. See
  [Performance](../reports/PERFORMANCE_RESULTS.md#provider-transports).
