<p align="center">
  <img src="assets/jeco-256.png" width="128" alt="Jeco, the steel-blue Jecode gecko">
</p>

<p align="center">
  <img src="assets/wordmark-steel.svg" width="280" alt="Jecode">
</p>

<p align="center"><strong>Your code. Your loop.</strong></p>

<p align="center">
  A focused terminal coding agent with visible tool use, durable sessions, and
  one controller that stays under your control.
</p>

<p align="center">
  <a href="https://github.com/giovannijecha/jecode/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/giovannijecha/jecode/actions/workflows/ci.yml/badge.svg"></a>
  <a href="LICENSE"><img alt="MIT License" src="https://img.shields.io/badge/license-MIT-669BD2"></a>
  <img alt="Node.js 22.18+ or 24+" src="https://img.shields.io/badge/node-22.18%2B%20%7C%2024%2B-86CB92">
  <img alt="Runtime dependencies: zero" src="https://img.shields.io/badge/runtime_dependencies-0-8DB4DD">
</p>

<p align="center">
  <a href="https://github.com/giovannijecha/jecode/blob/main/CHANGELOG.md">Changelog</a> &middot;
  <a href="https://github.com/giovannijecha/jecode/blob/main/docs/COMPATIBILITY.md">Compatibility</a> &middot;
  <a href="https://github.com/giovannijecha/jecode/releases">Releases</a>
</p>

> Jecode is currently pre-1.0. The core loop is usable today, but commands and
> terminal interactions may still evolve before 1.0. The release-candidate
> surface is now frozen in the [compatibility contract](https://github.com/giovannijecha/jecode/blob/main/docs/COMPATIBILITY.md).

## Why Jecode

Jecode keeps the coding loop yours. Work stays in one terminal where you can
follow tool calls and diffs, steer the model while it runs, and resume the same
conversation later. One controller carries each turn from prompt to result—no
delegated agents or hidden model workers.

Choose Anthropic, OpenAI, an eligible ChatGPT account, or Ollama without
changing the workflow. Jecode is written in TypeScript and released as plain
JavaScript for Node.js, with no installation scripts and zero third-party
runtime dependencies. The runtime stays small enough to inspect, understand,
and change.

## Quick start

Jecode requires **Node.js 22.18+ on the 22.x line, or Node.js 24+**, and npm.

### Install or update

Install the current stable release. Running the same command again updates an
existing installation:

```console
npm install --global @giovannijecha/jecode@latest
```

Confirm the installed version:

```console
jecode --version
```

### Start Jecode

Open the project directory you want Jecode to work on, then start Jecode:

```console
jecode
```

Jecode opens directly on an empty composer. Use `/providers` to connect a
service, `/models` to choose a model, then describe the work you want done.

```text
Review this project, explain its architecture, and propose the smallest safe
change to improve startup performance.
```

Use `jecode --root path/to/project` to select another workspace, or
`jecode --ephemeral` when the conversation must stay memory-only.

### Resume a conversation

Open the searchable resume picker for the current workspace:

```console
jecode resume
```

Resume the most recently updated conversation directly:

```console
jecode resume --latest
```

List every startup option:

```console
jecode --help
```

Windows, Ubuntu, and macOS are covered by the project test matrix.

### Uninstall

```console
npm uninstall --global @giovannijecha/jecode
```

Uninstalling the command preserves `~/.jecode`. Remove that directory only when
you intentionally want to erase saved settings, credentials, accounts, and
sessions.

### Linux and WSL

WSL uses its own Node.js installation and `PATH`; the Node.js version installed
on Windows does not apply inside it. Keep user-installed npm commands in the
Linux user path. Set the user-level npm prefix:

```console
npm config set prefix "$HOME/.local"
```

Add it to the current shell's `PATH`:

```console
export PATH="$HOME/.local/bin:$PATH"
```

Install or update Jecode:

```console
npm install --global @giovannijecha/jecode@latest
```

Refresh the command cache:

```console
hash -r
```

Confirm which executable will run:

```console
command -v jecode
```

Verify the installed version:

```console
jecode --version
```

Persist the `PATH` export in `~/.profile` or your shell's startup file. Inside
WSL, `command -v jecode` should resolve below `/home/...`, not through an
inherited Windows path below `/mnt/c`. Do not ignore `EBADENGINE`: `node
--version` must report 22.18+ on the 22.x line, or 24+.

## Documentation

The [user guide](https://github.com/giovannijecha/jecode/blob/main/docs/USAGE.md)
covers provider access, TUI controls, sessions and compaction, batch mode,
configuration, and safety boundaries.

## Community

- Ask questions, share workflows, and explore early ideas in
  [GitHub Discussions](https://github.com/giovannijecha/jecode/discussions).
- Report reproducible bugs and focused feature requests through
  [GitHub Issues](https://github.com/giovannijecha/jecode/issues). New
  capabilities are deferred until the 1.0 stabilization cycle is complete.
- Report security concerns privately through the repository Security tab.

Public pull requests are not accepted at this stage. Code changes remain a
maintainer and invited-collaborator workflow. See
[CONTRIBUTING.md](https://github.com/giovannijecha/jecode/blob/main/CONTRIBUTING.md)
and
[CODE_OF_CONDUCT.md](https://github.com/giovannijecha/jecode/blob/main/CODE_OF_CONDUCT.md).

## License

Jecode is available under the [MIT License](LICENSE).
