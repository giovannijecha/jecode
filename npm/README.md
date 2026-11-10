# Jecode

Jecode is a coding agent for your terminal. Choose a model, describe a task,
and let it read your project, edit files, run commands and check
the result. Tool cards and diffs show the work as it happens.

![Jecode running in Windows Terminal, with its startup screen, composer and model status](https://raw.githubusercontent.com/giovannijecha/jecode/main/docs/images/jecode-startup.png)

*Jecode 0.4.5 in Windows Terminal, with GPT-6.1 Sol selected.*

## Install

Install Jecode with Node.js 22 or later and npm:

```sh
npm install -g @giovannijecha/jecode
jecode --version
```

The npm package supports Windows 10 1809 or later (x64), Linux x64 with glibc 2.28
or newer, and macOS 11 or later on Apple silicon or Intel. It includes the platform
executables; it does not build code or fetch a binary when you first run it.
Node.js starts the npm command. A standalone build of Jecode does not need Node.js.
On macOS 11–13.4, use Node.js 22; Node.js 24 needs macOS 13.5 or later.

## Start working

Run `jecode` in your project, open `/settings` and choose **Providers**. Connect
an OpenRouter, OpenAI or Anthropic API key, or sign in with ChatGPT, then select
a model. A Claude subscription compatibility route is also available; read the
[provider caveat](https://github.com/giovannijecha/jecode/blob/main/docs/usage/PROVIDERS.md#claude-account).
API keys use API billing separately from subscription plans.

Press Enter to send a task, Ctrl+G to open commands while keeping your draft,
Esc to stop a turn, and Ctrl+Q to quit. `/help` or F1 lists commands and common
shortcuts, and `jecode resume` continues saved work. Read the
[terminal guide](https://github.com/giovannijecha/jecode/blob/main/docs/tui/INTERFACE.md)
for themes, tool cards and shortcuts.

The default permission level, Workspace, confines the agent's commands to your
project and asks before touching anything outside it; see
[Permissions](https://github.com/giovannijecha/jecode/blob/main/docs/usage/PERMISSIONS.md)
for what each system supports.

Jecode sends your conversation, including file contents and command output the
agent has read, to the provider you select. Your keys, account tokens and
conversations are stored locally under `~/.jecode/`.

The harness is one native executable written in Rust without third-party Rust
crates. It supports batched file tools and prepared tool sequences in one request.

## Long conversations and processes

Each request adds to the previous one, keeping earlier messages unchanged.
When a request would pass
the **Auto-compact at** threshold (default 80% of the model's window), or the
provider rejects it as too long, the same model writes one summary of the
earlier work. The original conversation stays saved and available to the model
by reference. Read [Long-running work](https://github.com/giovannijecha/jecode/blob/main/docs/usage/LONG_WORK.md).

`/process` lists running and recent commands, with saved output and a control to
move a waiting call into the background. Process completions reach the model and
conversation while the status row keeps the current activity and final turn
statistics visible.

## Update or remove

This build resumes only conversations saved in its current format; it reports
older ones and leaves their files unchanged, without migrating them. Read the
[release notes](https://github.com/giovannijecha/jecode/blob/main/CHANGELOG.md)
before updating, especially the
[0.5.5 upgrade notes](https://github.com/giovannijecha/jecode/blob/main/CHANGELOG.md#upgrading-from-045). The
[0.4.5 notes](https://github.com/giovannijecha/jecode/blob/main/CHANGELOG.md#upgrading-from-035)
cover the Claude prompt-cache default.

Update the npm installation with:

```sh
npm update -g @giovannijecha/jecode
```

Remove it with:

```sh
npm uninstall -g @giovannijecha/jecode
```

Removing the package keeps your settings, accounts and conversations in
`~/.jecode/` or your `JECODE_HOME` directory. If you previously
installed Jecode from source, check which `jecode` command appears first on
`PATH`; npm does not remove that separate installation.

See the [installation guide](https://github.com/giovannijecha/jecode/blob/main/docs/usage/INSTALLATION.md)
for source installation and troubleshooting, the [command-line guide](https://github.com/giovannijecha/jecode/blob/main/docs/usage/CLI.md)
for usage, and the [release notes](https://github.com/giovannijecha/jecode/blob/main/CHANGELOG.md)
for what changed.
