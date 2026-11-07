# Contributing

Small, focused tweaks and documentation improvements can go straight into a pull
request. For bug fixes, new features, substantial refactors, or changes to the
interface or architecture, open an issue first so the maintainer can agree on the
problem and approach before implementation. Use the bug report or feature request
form as appropriate, and link the agreed issue from the pull request.

Contributions are reviewed and may need changes and further verification before
they are accepted. Keep pull requests focused and explain the behavior a reviewer
should check.

## Ground rules

- Jecode is one Cargo package using the Rust standard library and original
  project code. Do not add external crates, copied implementations or
  third-party runtime libraries.
- npm distribution may use an original Node.js launcher and packaging tools;
  do not add npm dependencies or make the standalone Rust executable depend on
  Node.js.
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
[CI](docs/development/TESTING.md#ci). Windows and Linux run on maintainer-hosted
runners that reject fork code before any step runs, so on a fork's pull request
those checks do not verify your change. Run the normal checks locally and report
the results. Approving a GitHub workflow does not bypass the local runners' fork
policy. After reviewing an external contribution, the maintainer can bring the
approved changes onto a branch in this repository and open a linked internal pull
request for the full four-platform CI. Once that pull request is verified and
merged, the external pull request is closed with a link to the integration.

Opening a pull request grants no write or merge permission. Integration requires
the maintainer's decision and all required checks passing on the final revision.

Report vulnerabilities privately as described in [SECURITY.md](SECURITY.md).
