# Platform support

The first npm alpha packages Windows x64 only. Linux x64 is distributed as a
native archive on GitHub Releases; macOS has no supported artifact yet.

The supported execution targets are Windows and Linux. The exercised local
environments are Windows Terminal with GNU LLVM Rust and Ubuntu under WSL.
The public CI matrix checks native Windows and Ubuntu builds. A configured CI job
does not itself establish that a particular commit has passed remotely.

Windows MSVC builds require the Visual C++ linker. GNU LLVM builds use the
`x86_64-pc-windows-gnullvm` toolchain with LLVM-MinGW; `.cargo/config.toml` links
compiler runtime support statically for that target.

Linux workspace resolution requires Linux 5.6+ `openat2` and `/proc/self/fd`.
Explicit external local paths may select another filesystem; symlinks and magic
links remain forbidden. Bounded workspace opens also reject mount crossings.
File changes require supported no-replace native operations. Some Windows-backed
WSL mounts refuse them; use an ordinary Linux filesystem for those operations.
User JSON storage additionally requires private permissions and file locking.
Filesystems without the required semantics fail rather than weakening the boundary.

Native terminal and workspace bindings cover the implemented Windows and Linux
ABIs. Other operating systems, including macOS, are not advertised as supported.
Information commands work with redirected output; interactive conversations need
a supported terminal. Unknown CLI options return status 2 without echoing their
contents.

OpenAI Account is the current provider. Device sign-in must be enabled for the
account, and model availability depends on that account. The connected choices
are GPT-5.6 Luna and Terra, with medium effort. This is an experimental account
protocol, not a guarantee of compatibility with every account or future server.

Networking currently uses HTTP/1.1 over the owned TLS profile. There is no HTTP/2,
proxy configuration, web browsing, external provider plugin system or stable
public Rust library API. See [TLS](TLS.md) and [usage](USAGE.md) for other limits.
