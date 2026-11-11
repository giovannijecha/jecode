# Jecode

[![CI](https://github.com/giovannijecha/jecode/actions/workflows/CI.yml/badge.svg)](https://github.com/giovannijecha/jecode/actions/workflows/CI.yml)

Jecode is a coding agent for your terminal. You describe a task; the model you
choose reads your project, edits files, runs commands and checks
the result, while you watch each step and control what it may change.

It is a single native executable written in Rust, without third-party Rust
crates. Connect an OpenRouter, OpenAI or Anthropic API key, or sign in with
your ChatGPT account. A Claude subscription compatibility route is also
available; read the [provider caveat](docs/usage/PROVIDERS.md#claude-account).

![Jecode running in Windows Terminal, with its startup screen, composer and model status](docs/images/jecode-startup.png)

*Jecode 0.5.5 in Windows Terminal, with GPT-6.1 Sol selected.*

## Long conversations and processes

Requests only grow until they near the configured share of the context window;
then the model summarizes earlier work and keeps recent messages verbatim. The
full conversation stays saved, and the model can retrieve the originals by
reference. See [Long-running work](docs/usage/LONG_WORK.md).

Use `/process` to inspect running and recent commands, read their saved output
or move a waiting call into the background. Completed commands report their
outcome to the model and conversation while the status row keeps the current
activity and final turn statistics visible.

## Work in your project

Start `jecode` in a project directory and type a request, such as *"The tests in
`src/parser` fail since the last change. Find out why and fix it."* The agent
then works through the task in the same window:

- each file it reads, search it runs and command it executes appears as a compact
  card, and edits appear as diffs;
- you can keep typing to add instructions, which it picks up before its next
  step, or press Esc to stop it;
- at the default permission level, it asks you before touching anything outside
  the project directory;
- `/` lists commands as you type, and Ctrl+G opens them without touching your
  draft;
- Auto, Dark and Light themes keep your terminal's own background.

When you quit, Jecode prints a summary and the command to continue later:

```text
>_ jecode closed · 3 prompts · 12 tools · 4m 12s
   jecode resume 1793721600123-4812-0
```

You can also run one task without the fullscreen interface, or pipe one in; see
[Command line](docs/usage/CLI.md):

```sh
jecode "Explain the structure of this project"
```

## Install

Install Jecode with Node.js 22 or later and npm on Windows 10 1809 or later (x64),
Linux x64 (glibc 2.28 or newer), or macOS 11 or later on Apple silicon or Intel.
On macOS 11–13.4, use Node.js 22: Node.js 24 needs macOS 13.5 or later.
[Platform support](docs/usage/PLATFORMS.md#supported-versions) records what
has actually run on each of them.

```sh
npm install -g @giovannijecha/jecode
jecode --version
```

The npm command launches a bundled native executable. It needs Node.js to
start; the standalone executable does not. To build and install from source,
see [Installation](docs/usage/INSTALLATION.md). That guide also
covers npm updates, removal, and existing standalone installations.

## First task

1. Open a terminal in the project you want to work on and run `jecode`.
2. Connect a provider: type `/settings`, choose **Providers**, then either
   **API keys** → **OpenRouter**, **OpenAI API** or **Anthropic API** and paste
   that provider's key. **Credentials** offers ChatGPT sign-in and the
   documented Claude compatibility route.
3. Choose a model when the selector opens. This saves your configuration.
4. Type your task and press Enter. The default level, Workspace, confines the
   agent's commands to this folder; see [Permissions](docs/usage/PERMISSIONS.md).

Instead of steps 2 and 3, you can run `jecode setup` for a guided prompt.

To continue later, run `jecode resume` from the same directory. Press Ctrl+Q to
quit, and `/help` or F1 for commands and common shortcuts.

## Learn more

- [Providers](docs/usage/PROVIDERS.md): account setup, usage and provider-specific
  compatibility limits.
- [Terminal interface](docs/usage/INTERFACE.md): sending, queueing, stopping and
  reading the agent's work.
- [Permissions](docs/usage/PERMISSIONS.md): what the agent may do, and approvals.
- [MCP connections](docs/usage/MCP.md): connect remote tools from Settings.
- [Language servers](docs/usage/LSP.md): query definitions, references and diagnostics.
- [Project instructions](docs/usage/PROJECT_INSTRUCTIONS.md): give the agent
  your project's rules with `AGENTS.md`.
- [Documentation index](docs/README.md): every guide and reference.

## Status

Jecode is usable and still evolving. Commands, settings and storage formats may
change between releases; the [release notes](CHANGELOG.md) list what changed.
This build resumes only conversations saved in its current format; it reports
older ones and leaves their files unchanged, without migrating them. See
[Sessions](docs/usage/SESSIONS.md) and the
[0.5.5 upgrade notes](CHANGELOG.md#upgrading-from-045). The
[0.4.5 notes](CHANGELOG.md#upgrading-from-035) cover the Claude prompt-cache
default.

- Windows x64, Linux x64, and macOS on Apple silicon and Intel have native
  builds and terminal fixtures in CI. Interactive use on macOS has not been
  verified. See [Platform support](docs/usage/PLATFORMS.md).
- Command confinement in the Workspace permission level needs Landlock on Linux and a
  recent Windows, and uses Seatbelt on macOS. Hosts without confinement need the
  Full level; macOS terminal hosts still need manual validation.
- API keys use the provider's API billing, separately from subscription plans.

## Privacy and security

Jecode sends your conversation, including file contents and command output the
agent has read, to the provider you select. Your API keys, account
tokens and conversations are stored locally under `~/.jecode/`. See
[SECURITY.md](SECURITY.md) for details and how to report a vulnerability.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) and
[Development and testing](docs/development/TESTING.md).

Jecode is released under the [MIT license](LICENSE).
