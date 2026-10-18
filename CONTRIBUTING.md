# Contributing

Open an issue for a bug or a concrete feature proposal. Discuss substantial
changes before implementing them. Keep pull requests focused and explain the
behavior a reviewer should check.

## Development

Use Rust 1.95.0, pinned by `rust-toolchain.toml`, and the native tools listed
in [README.md](README.md). Jecode uses one Cargo package, the Rust standard
library and original project code. Do not add external crates, copied
implementations or third-party runtime libraries.

Run the [normal checks](docs/development/TESTING.md#normal-checks) from the
repository root. The testing guide also covers Windows linker selection,
focused checks and native terminal fixtures.

Use isolated fixtures for provider, process, clipboard and session checks.
Never include real keys, configuration, transcripts or personal attachments
in a contribution. See [TESTING.md](docs/development/TESTING.md) for native
checks and their limits.

## Pull requests

Write code, comments, documentation and commit messages in English. Preserve
the CLI and terminal interfaces. Keep modules cohesive and around 500 lines
or fewer; split by responsibility when necessary.

CI runs the standard checks on Windows, Linux and macOS; see
[CI](docs/development/TESTING.md#ci) for runner requirements. State which
platforms you tested and identify any native or manual checks still needed.

Submit changes through a pull request to `main`. Explain the problem,
resulting behavior, tests actually run and relevant limitations. The
maintainer reviews and merges changes after required checks pass.

Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md).
