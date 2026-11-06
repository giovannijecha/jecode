# Jecode

Jecode is a fast, efficient coding agent for your terminal. Choose a model,
describe a task, and let it read your project, edit files, run commands and check
the result. Tool cards and diffs show the work as it happens.

![Jecode running in Windows Terminal, with its startup screen, composer and model status](https://raw.githubusercontent.com/giovannijecha/jecode/main/docs/images/jecode-startup.png)

*Windows Terminal capture on 0.2.5, before the 0.3.5 version bump.*

## Measured efficiency

In our five-turn coding benchmark, Jecode used **21% fewer cost-weighted tokens
than Codex** with GPT-6.1 Sol and **36% fewer than Claude Code** with Claude
Sonnet 5.5. All runs passed 21/21 hidden acceptance tests.

| Same model, high effort | Weighted token use | Active time |
| --- | ---: | ---: |
| Jecode vs Codex 0.166, GPT-6.1 Sol | **21% less** | 11% longer |
| Jecode vs Claude Code 2.1.318, Sonnet 5.5 | **36% less** | **30% shorter** |

One Node project, Windows, 2026-11-04: 7 vs 6 GPT runs and 4 vs 3 Claude runs.
Jecode used account sign-in; its API-key routes were not compared.
Time excludes two deliberate 6.5-minute pauses. Weighted
tokens are a cache-aware cost proxy, not raw totals, API bills or plan quota.
See the [method, per-run data and limits](https://github.com/giovannijecha/jecode/blob/main/docs/development/AGENT_BENCHMARK.md).

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
Esc to stop a turn, and Ctrl+Q to quit. `jecode resume` continues saved work.
Read the [terminal guide](https://github.com/giovannijecha/jecode/blob/main/docs/tui/INTERFACE.md)
for the dark and light themes, tool cards, checkpoints and shortcuts.

The harness is one native executable written in Rust without third-party Rust
crates. It supports prompt caching, batched file tools and prepared tool
sequences in one request. Native HTTP connections persist between requests;
OpenAI conversations use WebSocket with HTTP fallback. Separate
[performance measurements](https://github.com/giovannijecha/jecode/blob/main/docs/development/PERFORMANCE.md)
record lower first-update latency on Windows; the comparison above predates
that work.

## Update or remove

Update the npm installation with:

```sh
npm update -g @giovannijecha/jecode
```

Remove it with:

```sh
npm uninstall -g @giovannijecha/jecode
```

Removing the package keeps your settings, accounts, conversations, and
checkpoints in `~/.jecode/` or your `JECODE_HOME` directory. If you previously
installed Jecode from source, check which `jecode` command appears first on
`PATH`; npm does not remove that separate installation.

See the [installation guide](https://github.com/giovannijecha/jecode/blob/main/docs/usage/INSTALLATION.md)
for source installation and troubleshooting, and the [command-line guide](https://github.com/giovannijecha/jecode/blob/main/docs/usage/CLI.md)
for usage.
