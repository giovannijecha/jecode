# Installation

Jecode is a single executable. You build it from source, optionally package it,
and install it with the included installer script. No prebuilt downloads are
published yet.

## Requirements

### Running Jecode

| Platform | Required | Needed only for some features |
| --- | --- | --- |
| Windows 10 or later | The native console. | Windows PowerShell 5.1 for `/copy` and clipboard images (Alt+V). ConPTY, included in Windows 10 and later, for interactive commands. |
| Linux | A VT-compatible terminal. The system OpenSSL libraries (`libssl.so.3` and `libcrypto.so.3`, or the 1.1 versions) for HTTPS. | `libX11.so.6` or `libwayland-client.so.0` for clipboard images, GdkPixbuf for image conversion, and a terminal that accepts OSC 52 for `/copy`. |
| macOS | A VT-compatible terminal. | None beyond the system tools. |

The `browser` tool needs Google Chrome or Microsoft Edge, or another browser
named by `JECODE_BROWSER`; see [Tools](TOOLS.md#web-and-browser). Commands that
the agent runs for your project, such as `cargo` or `npm`, need that project's
own toolchain. Jecode never installs or downloads programs to enable its tools.

Verification differs by platform. Windows is the primary development platform;
Linux and macOS are built and tested in CI. See
[Platform support](../development/PLATFORMS.md) before relying on Linux or
macOS, and note that on those platforms commands require the Full
[permission level](PERMISSIONS.md#platform-limits).

### Build prerequisites

- **Git.** Needed to clone the source repository.
- **Rust.** Install [rustup](https://rustup.rs/). The repository pins Rust
  1.95.0, which rustup installs on first use.
- **Windows.** A Rust linker: either the Visual Studio C++ build tools for the
  default MSVC toolchain, or the GNU LLVM toolchain described in
  [Development and testing](../development/TESTING.md#windows-toolchain). The
  build also uses the .NET Framework 4 compiler included with Windows at
  `%SystemRoot%\Microsoft.NET\Framework64\v4.0.30319\csc.exe`.
- **Linux.** A C compiler available as `cc` (or named by `CC`).
- **macOS.** Apple's Command Line Tools (`xcode-select --install`).

These are build requirements only: the native adapters are compiled into the
executable, so running Jecode needs no compiler.

## Build from source

Clone the repository and build a release executable:

```sh
git clone https://github.com/giovannijecha/jecode.git
cd jecode
cargo build --release --locked --offline
```

Once the pinned Rust toolchain and platform prerequisites are installed, the
build runs offline. The result is
`target/release/jecode` (`target\release\jecode.exe` on Windows). Check it:

```sh
./target/release/jecode --version
```

The output looks like `jecode 0.1.0 (linux-x86_64-release, exe fnv1a64:…)`: the
version, the platform and build profile, and a fingerprint of the executable
that helps tell builds apart.

## Install

The installers copy a packaged executable into a per-user directory. They never
build source, so prepare a package first.

### Windows

In PowerShell, from the repository root:

```powershell
.\PACKAGE.ps1
.\target\package\jecode-windows-x86_64\INSTALL.ps1
```

`PACKAGE.ps1` builds the release executable and prepares
`target\package\jecode-windows-<arch>` containing `jecode.exe`, `INSTALL.ps1`,
the license, the README, the documentation and a `SHA256SUMS` file.
`INSTALL.ps1` then:

1. Checks that the packaged executable starts.
2. Copies it to `%LOCALAPPDATA%\Jecode\bin\jecode.exe`, replacing an earlier
   Jecode installation there but refusing to overwrite an unrelated file.
3. Puts that directory first in your user `PATH` and in the current terminal.

When it prints `Installed: …` and `This terminal is ready`, run `jecode
--version` to confirm. Open new terminals to pick up the updated `PATH`.

| Option | Effect |
| --- | --- |
| `-InstallRoot C:\absolute\path` | Install into `<path>\bin` instead of `%LOCALAPPDATA%\Jecode\bin`. |
| `-NoPath` | Leave `PATH` unchanged; run the installed executable by its full path. |

If the installer warns that another command overrides `jecode`, an earlier entry
on `PATH` wins. Run the installed executable by its full path or remove the
other entry.

### Linux and macOS

From the repository root:

```sh
sh PACKAGE.sh
```

Run the installer for the package you built:

**Linux x86_64:**

```sh
sh target/package/jecode-linux-x86_64/INSTALL.sh
```

**macOS on Apple silicon:**

```sh
sh target/package/jecode-macos-aarch64/INSTALL.sh
```

**macOS on Intel:**

```sh
sh target/package/jecode-macos-x86_64/INSTALL.sh
```

`PACKAGE.sh` prints the exact output directory. The installer copies the
executable to
`${XDG_DATA_HOME:-$HOME/.local/share}/jecode/bin/jecode` and does not change
`PATH`. Add that directory to your shell configuration, for example in
`~/.profile`:

```sh
export PATH="${XDG_DATA_HOME:-$HOME/.local/share}/jecode/bin:$PATH"
```

To install elsewhere, pass an absolute directory your user can write to.
For example, using the Linux x86_64 package:

```sh
sh target/package/jecode-linux-x86_64/INSTALL.sh "$HOME/.local/opt/jecode"
```

### Run without installing

A package directory works on its own: run `jecode` (or `jecode.exe`) from it
directly, or run `target/release/jecode` after building.

## Update

Pull the newer source, then repeat the package and install steps. The installer
replaces the executable it installed earlier. Your configuration, accounts and
conversations in `~/.jecode/` are not touched.

## Uninstall

1. Delete the install directory: `%LOCALAPPDATA%\Jecode` on Windows, or
   `${XDG_DATA_HOME:-$HOME/.local/share}/jecode` on Linux and macOS (or the
   root you chose).
2. Remove its `bin` directory from your `PATH`. On Windows, edit the user `Path`
   variable in **Settings → System → About → Advanced system settings →
   Environment Variables**.
3. Optionally delete `~/.jecode/` (`%USERPROFILE%\.jecode\` on Windows). It holds
   your configuration, OpenRouter key, account tokens, saved conversations and
   checkpoints; see [Configuration](CONFIGURATION.md#files-jecode-keeps).
