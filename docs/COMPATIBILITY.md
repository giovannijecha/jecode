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
account. Jecode requests a bounded model catalog from the signed-in account's
Codex backend, then offers listed models and their advertised reasoning efforts.
The account catalog is distinct from the public API-key `/v1/models` endpoint.
Catalog entries do not guarantee that a subsequent generation request will be
accepted for a particular account. If catalog retrieval fails, Jecode keeps the
current selection and allows a retry through `/model`.

The catalog route and response fields were checked against the official
[OpenAI Codex source](https://github.com/openai/codex/tree/30fc6864cc1318121eca1843c217fe00ce1212f1/codex-rs)
at revision `30fc6864cc1318121eca1843c217fe00ce1212f1` (2026-09-23):
`GET https://chatgpt.com/backend-api/codex/models?client_version=0.156.1`,
with account authentication; `slug`, `visibility`, `default_reasoning_level`
and `supported_reasoning_levels` describe choices. The pinned client version is
a compatibility query matching the official
[0.156.1 release](https://github.com/openai/codex/releases/tag/rust-v0.156.1)
published on 2026-09-23, not Jecode's version or a public contract for
independent clients. Live account behavior and future server compatibility
have not been validated for this change.

Jecode's provider networking currently uses HTTP/1.1 over the owned TLS profile.
There is no HTTP/2, proxy configuration, dedicated web search,
browser automation integration, external provider plugin system
or stable public Rust library API. The command runner can attempt network work
through available local programs; its results establish what actually worked.
When explicit account catalog metadata permits it, `view_image` sends local PNG
evidence using an `input_image` item in a function-call output. This wire shape
and image modality selection are source-backed by [Codex at
`86be5320`](https://github.com/openai/codex/blob/86be5320b068ef67b56348b02aa8c33706955da6/codex-rs/core/src/tools/handlers/view_image.rs).
Jecode's end-to-end visual understanding still needs a maintainer-run live account
test; offline fixtures validate encoding, storage and lifecycle only.
See [TLS](TLS.md), [tools](TOOLS.md) and [usage](USAGE.md) for other limits.
