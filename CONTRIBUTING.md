# Contributing

Open an issue for a bug or a concrete feature proposal. Discuss substantial
changes before implementing them. Keep pull requests focused and explain the
behavior a reviewer should check.

## Development

Use Rust 1.95.0, pinned by `rust-toolchain.toml`, and the native tools listed
in [README.md](README.md). Jecode uses one Cargo package, the Rust standard
library and original project code. Do not add external crates, copied
implementations or third-party runtime libraries.

Run the applicable checks from the repository root:

```text
cargo build --locked --offline
cargo fmt --all -- --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline
```

Use isolated fixtures for provider, process, clipboard and session checks.
Never include real keys, configuration, transcripts or personal attachments
in a contribution. See [TESTING.md](docs/development/TESTING.md) for native
checks and their limits.

## Pull requests

Write code, comments, documentation and commit messages in English. Preserve
the CLI and terminal interfaces. Keep modules cohesive and around 500 lines
or fewer; split by responsibility when necessary.

CI runs the standard gates on Windows, Linux and macOS. On Windows it uses
the pinned gnullvm host and Rust's bundled `rust-lld` linker with the
`rust-mingw` component. A passing job covers the executed tests; ignored
native checks and physical terminal interactions remain separate verification.

Submit changes through a pull request to `main`. Explain the problem,
resulting behavior, tests actually run and relevant limitations. The
maintainer reviews and merges changes after required checks pass.

Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md).
