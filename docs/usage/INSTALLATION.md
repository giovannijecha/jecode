# Installation

Jecode can be installed through npm or as a standalone executable built from
source. The first npm package and prebuilt downloads have **not been published
yet**. The npm commands below describe the prepared release; use the source
instructions until it is published.

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

## Install with npm (after publication)

The npm package supports Windows x64, Linux x64 with glibc 2.39 or newer, and
macOS on Apple silicon. Its macOS build is verified on macOS 15; earlier
versions have not been tested. Install Node.js 22 or later and npm, then run:

```sh
npm install -g jecode
jecode --version
```

The version command should print a line beginning with `jecode 0.1.0` for the
first release. The package includes the native executables and does not build
Rust code or download a binary when you first run it. Node.js starts the npm
command; the standalone executable below needs no Node.js.

If a global npm install fails with a permissions error, follow
[npm's user-owned installation guidance](https://docs.npmjs.com/resolving-eacces-permissions-errors-when-installing-packages-globally/).

### Update or remove the npm installation

To update:

```sh
npm update -g jecode
jecode --version
```

To remove:

```sh
npm uninstall -g jecode
```

npm removes its own package and command. Your settings, accounts, conversations,
and checkpoints in `~/.jecode/` (or the directory selected by `JECODE_HOME`)
remain in place. An npm uninstall does not remove a separate source-built
installation. On Windows, close any running Jecode process before updating or
uninstalling if npm reports that the executable is in use.

### Existing standalone installations

An earlier source-built `jecode` may be found before npm's command on `PATH`.
Check which command runs:

**Windows (PowerShell):**

```powershell
Get-Command jecode -All
```

**Linux and macOS:**

```sh
command -v jecode
```

If that resolves to the standalone install directory instead of npm's command,
remove only that directory's `bin` entry from your `PATH`, or use the npm
command by its resolved path. npm does not delete the older installation or
change your shell configuration. See [Remove a standalone installation](#remove-a-standalone-installation)
if you no longer need it.

## Build and install from source

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

### Build a standalone executable

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

### Install the standalone executable

The installers copy a packaged executable into a per-user directory. They never
build source, so prepare a package first.

#### Windows

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

#### Linux and macOS

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

#### Run without installing

A package directory works on its own: run `jecode` (or `jecode.exe`) from it
directly, or run `target/release/jecode` after building.

### Update a standalone installation

Pull the newer source, then repeat the package and install steps. The installer
replaces the executable it installed earlier. Your configuration, accounts and
conversations in `~/.jecode/` are not touched.

### Remove a standalone installation

1. Identify the root you used for the standalone installer (by default,
   `%LOCALAPPDATA%\Jecode` on Windows or
   `${XDG_DATA_HOME:-$HOME/.local/share}/jecode` on Linux and macOS). Check
   its `.jecode-install` marker and remove its `bin/jecode` executable
   (`bin\jecode.exe` on Windows) and marker. Remove empty directories afterward;
   keep any unrelated files in a custom install root.
2. Remove only this standalone `bin` directory from your `PATH` or shell
   profile. On Windows, edit the user `Path` in **Settings → System → About →
   Advanced system settings → Environment Variables**.

This does not remove `~/.jecode/` (`%USERPROFILE%\.jecode\` on Windows), which
holds your configuration, credentials, saved conversations, and checkpoints;
see [Configuration](CONFIGURATION.md#files-jecode-keeps).
