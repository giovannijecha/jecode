# Contributing

Open an issue for a bug or a concrete feature proposal, and discuss substantial
changes before implementing them. Keep pull requests focused and explain the
behavior a reviewer should check.

## Ground rules

- Jecode is one Cargo package using the Rust standard library and original
  project code. Do not add external crates, copied implementations or
  third-party runtime libraries.
- Platform features use operating-system APIs through Jecode's own native
  adapters. Application features must not depend on separately installed
  programs such as Git or Git Bash.
- Jecode helps the model through tools, context and history. It does not add
  hardcoded refusals, completion gates or validation that second-guesses the
  model's output.
- Keep the agent loop independent of terminal rendering, and keep the CLI for
  single prompts, pipes and `--plain` working.
- Keep modules cohesive and source files around 500 lines or fewer; split by
  responsibility when they grow.
- Write code, comments, documentation and commit messages in English. Markdown
  file names are uppercase.

## Development

Use Rust 1.95.0, pinned by `rust-toolchain.toml`, and the build prerequisites in
[Installation](docs/usage/INSTALLATION.md#build-prerequisites). Before opening a
pull request, run the [normal checks](docs/development/TESTING.md#normal-checks)
from the repository root:

```text
cargo build --locked --offline
cargo fmt --all -- --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline
```

Run the native terminal fixture for your platform when you change input,
rendering, resize or terminal restoration; see
[Development and testing](docs/development/TESTING.md).

Use isolated fixtures for provider, process, clipboard and session checks, and
an absolute scratch `JECODE_HOME` for manual runs. Never include real keys,
configuration, conversations or personal attachments in a contribution.

## Pull requests

Submit changes through a pull request to `main`. Describe the problem, the
resulting behavior, the checks you ran and any limitations. State which
platforms you tested and which native or manual checks are still needed.

CI runs the checks on Windows, Linux and macOS; see
[CI](docs/development/TESTING.md#ci). The maintainer reviews and merges changes
after the required checks pass.

Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md).
