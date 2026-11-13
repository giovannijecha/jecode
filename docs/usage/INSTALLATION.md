# Installation

Install Jecode with npm, or build a standalone executable from source.

## Requirements

### Running Jecode

Windows x64, Linux x64 and macOS on Apple silicon or Intel are supported.
See [Platform support](PLATFORMS.md#supported-versions) for minimum OS versions,
required system libraries and feature-specific requirements.

The `browser` tool needs Google Chrome or Microsoft Edge, or another browser
named by `JECODE_BROWSER`; see [Tools](TOOLS.md#web-and-browser). Commands that
the agent runs for your project, such as `cargo` or `npm`, need that project's
own toolchain. Jecode never installs or downloads programs to enable its tools.

All four native targets are built and tested in CI, including terminal fixtures.
Windows and Linux have recorded live-provider use; manual interactive use on a
Mac remains unverified. See [Platform support](PLATFORMS.md).
Where the system cannot confine commands, they require the Full
[permission level](PERMISSIONS.md#platform-limits).

## Install with npm

Install Node.js 22 or later and npm, using a version supported by your OS
(see [runtime requirements](PLATFORMS.md#runtime-and-feature-requirements)).
Then run:

```sh
npm install -g @giovannijecha/jecode
jecode --version
```

The command installs the current npm release, and `jecode --version` prints its
installed version, platform and build details. The package includes the native
executables and does not build Rust code or download a binary when first run.
Node.js starts the npm command; the standalone executable below needs no Node.js.

If a global npm install fails with a permissions error, follow
[npm's user-owned installation guidance](https://docs.npmjs.com/resolving-eacces-permissions-errors-when-installing-packages-globally/).
On Windows, if PowerShell reports that running scripts is disabled, use `npm.cmd` and
`jecode.cmd`, or allow local scripts with
`Set-ExecutionPolicy -Scope CurrentUser RemoteSigned`.

Next, connect a provider and start working: see
[First task](../../README.md#first-task).

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
change your shell configuration. Run `jecode uninstall` with the standalone
executable to remove it; see [Remove Jecode](#remove-jecode).

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

The output starts with `jecode` and the version from `Cargo.toml`, followed by
the platform and build profile and a fingerprint of the executable
that helps tell builds apart.

### Install the standalone executable

The installers copy a packaged executable into a per-user directory. They never
build source, so prepare a package first. Both package scripts refuse an existing
output directory; move or remove an earlier package before preparing a new one.

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

`PACKAGE.sh` prints the exact output directory. Both packaging scripts stop when
that directory already exists; remove it before packaging again. The installer
copies the executable to
`${XDG_DATA_HOME:-$HOME/.local/share}/jecode/bin/jecode` and does not change
`PATH`. Add that directory to your shell configuration, for example in
`~/.profile`:

```sh
export PATH="${XDG_DATA_HOME:-$HOME/.local/share}/jecode/bin:$PATH"
```

To install elsewhere, pass an absolute directory your user can write to; the
executable goes in its `bin` folder. For example, using the Linux x86_64 package:

```sh
sh target/package/jecode-linux-x86_64/INSTALL.sh "$HOME/.local/opt/jecode"
```

#### Run without installing

A package directory works on its own: run `jecode` (or `jecode.exe`) from it
directly, or run `target/release/jecode` after building. `jecode update` and
`jecode uninstall` do not manage such a copy.

## Update Jecode

```sh
jecode update --check    # report the installed and latest versions
jecode update            # install the latest release
jecode --version
```

`jecode update` works on the installation that is running, identified by where
its executable lives rather than by `PATH` order, so several installations can
coexist:

| Installation | How it is recognized | What `jecode update` does |
| --- | --- | --- |
| npm global | The executable is inside the `@giovannijecha/jecode` package of an npm prefix | Runs `npm install --global --prefix PREFIX @giovannijecha/jecode@VERSION` with the npm of the Node.js that started Jecode, else the first npm on `PATH` that is not Volta's shim, so npm keeps owning the package. |
| Volta | The package is in Volta's image directory | Runs `volta install @giovannijecha/jecode@VERSION`. |
| Standalone | `INSTALL.ps1` or `INSTALL.sh` wrote the `.jecode-install` marker beside its `bin` folder | Downloads the published release itself; needs no Node.js, npm, Rust, Git or shell. |
| Project dependency, source build, copied file | None of the above | Reports the location and changes nothing. Update it the way you installed it. |

The latest version is the `latest` tag of the npm package `@giovannijecha/jecode`,
the one published release. A standalone update downloads that release's
tarball and installs it only when the tarball matches the registry's
`sha512` integrity and the executable inside matches the SHA-256 in the
release's own manifest. The new executable is written beside the old one, must
report the expected version with `--version`, and only then takes its place.
Any failure before that point leaves the installed executable as it was.

On Windows a running program cannot be overwritten, but it can be renamed:
Jecode renames itself out of the way, puts the new executable in place, and a
hidden Windows PowerShell removes the old file after Jecode exits. An npm or
Volta update moves the running executable out of the package first, so the
package manager can replace the folder; if the package manager fails, the
executable is moved back. Other Jecode processes started from the same file
keep running until they exit; a later update removes what they leave.

Read the [release notes](../../CHANGELOG.md) before updating: a new build
resumes only conversations in its current format and leaves unsupported files
unchanged. Set `JECODE_UPDATE_REGISTRY` to an npm registry mirror's URL to
check and download from it instead of `https://registry.npmjs.org`. Use an
`https://` mirror: the `sha512` check compares the tarball with the mirror's own
metadata, so it cannot protect a plain `http://` connection. Releases include
executables for Windows x64, macOS and Linux x64; a standalone installation on
another platform reports that before downloading anything.

## Remove Jecode

```sh
jecode uninstall
```

It asks you to type `uninstall` to confirm; add `--yes` to run it without a
terminal. It removes only the running installation:

- **npm and Volta:** runs `npm uninstall --global --prefix PREFIX` or
  `volta uninstall`, which remove the package and its command.
- **Standalone:** removes `bin/jecode` (`bin\jecode.exe`), leftovers of earlier
  updates and the `.jecode-install` marker, then the `bin` folder and the root
  if they are empty. Other files in the root stay, and the command lists them.
  On Windows it also removes that `bin` folder from your user `PATH`, keeping
  every other entry and the value's type; open a new terminal afterwards. A
  `bin` folder that still holds other files stays on `PATH`. On
  Linux and macOS the installer never changed `PATH`, so remove any line you
  added to your shell profile yourself.

An installation Jecode does not manage is reported and left alone.
Uninstalling never removes another installation or the Jecode home,
`~/.jecode/` (`%USERPROFILE%\.jecode\` on Windows, or `JECODE_HOME`), which
holds your configuration, credentials and saved conversations. Delete that
folder yourself if you want them gone; see [Files Jecode keeps](../reference/FILES.md).

The package managers' own commands still work: `npm update -g
@giovannijecha/jecode` and `npm uninstall -g @giovannijecha/jecode`. On Windows,
close running Jecode processes first if npm reports that the executable is in
use. To update a standalone installation from source instead, repeat the
package and install steps above; the installer replaces the executable it
installed earlier.
