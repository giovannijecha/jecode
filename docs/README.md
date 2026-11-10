# Jecode documentation

New to Jecode? Start with the [project README](../README.md): it covers what
Jecode does, installation and a first task. Run Jecode commands from your
project directory; run Cargo commands and repository scripts from the Jecode
repository root.

## Get started

| Guide | Covers |
| --- | --- |
| [Installation](usage/INSTALLATION.md) | Requirements, building, installing, updating and uninstalling. |
| [Providers](usage/PROVIDERS.md) | OpenRouter, OpenAI and Anthropic API keys; ChatGPT sign-in and the Claude account compatibility route. |
| [Terminal interface](tui/INTERFACE.md) | Working in the fullscreen interface: sending, queueing, stopping, reading and quitting. |
| [Command line](usage/CLI.md) | Single prompts, pipes, plain chat, options and exit codes. |

## Work with Jecode

| Guide | Covers |
| --- | --- |
| [Permissions](usage/PERMISSIONS.md) | Read, Workspace and Full; approvals; platform limits. |
| [Project instructions](usage/PROJECT_INSTRUCTIONS.md) | `AGENTS.md` files and `/init`. |
| [Personal skills](usage/SKILLS.md) | Reusable procedures in `~/.jecode/skills`. |
| [Tools](usage/TOOLS.md) | What the agent can do: files, search, commands, processes, web, browser and MCP. |
| [MCP connections](usage/MCP.md) | Global HTTP connections, tokens, browser sign-in and remote tool permissions. |
| [Language servers](usage/LSP.md) | User-enabled local servers, semantic navigation, diagnostics and execution scope. |
| [Saved conversations](usage/SESSIONS.md) | Resuming, recovering and deleting conversations. |
| [Attachments](usage/ATTACHMENTS.md) | Files and clipboard images. |
| [Collaborating agents](usage/COLLABORATION.md) | Delegated assignments, isolated workspaces and review. |
| [Questions and plans](usage/QUESTIONS_AND_PLANS.md) | Answering the agent's questions and following its plan. |
| [Copy and export](usage/COPY_AND_EXPORT.md) | Copying answers and saving conversations as JSON. |
| [Temporary files](usage/TEMPORARY.md) | Each conversation's working area and its cleanup. |
| [Long-running work](usage/LONG_WORK.md) | Retries, the context meter, compaction summaries and original history. |

## Reference

| Reference | Contents |
| --- | --- |
| [Configuration](usage/CONFIGURATION.md) | Settings, `config.json`, routing, stored files, environment variables and proxies. |
| [Commands and keys](tui/COMMANDS.md) | Every slash command and keyboard control in the TUI. |
| [Tool activity](tui/TOOL_ACTIVITY.md) | Reading tool cards, Work chapters, diffs and Ctrl+O. |
| [Markdown display](tui/MARKDOWN.md) | How answers are formatted in the terminal. |
| [Tool reference](usage/TOOL_REFERENCE.md) | Exact tool arguments, defaults and limits. |
| [Command language](usage/SHELL.md) | Syntax and built-ins of the `bash` tool. |

## Project

| Page | Contents |
| --- | --- |
| [Release notes](../CHANGELOG.md) | What changed in each version, with upgrade notes. |
| [Agent benchmark](development/AGENT_BENCHMARK.md) | Measured efficiency against Codex and Claude Code, with per-run results and limits. |
| [Security](../SECURITY.md) | Stored data, credentials and reporting a vulnerability. |
| [Contributing](../CONTRIBUTING.md) | How to propose a change. |

## Development

| Reference | Contents |
| --- | --- |
| [Development and testing](development/TESTING.md) | Toolchain, checks, native fixtures, live checks and CI. |
| [Context checks](development/CONTEXT_CHECKS.md) | Offline replay of saved contexts, without model calls, and dated screening results. |
| [Releasing](development/RELEASING.md) | Preparing and verifying native and npm release artifacts. |
| [Architecture](development/ARCHITECTURE.md) | Modules, request lifecycle, storage and native boundaries. |
| [Platform support](development/PLATFORMS.md) | Verification status, platform limits and host checks. |
| [OpenAI backend](development/OPENAI.md) | Sign in with ChatGPT and the Responses route. |
| [Anthropic backend](development/ANTHROPIC.md) | The Claude account compatibility route. |
| [GitHub MCP App](development/GITHUB_MCP_APP.md) | App registration and public client ID for device sign-in. |
| [Vercel MCP client](development/VERCEL_MCP_CLIENT.md) | Public OAuth metadata, hosting and Jecode client setup. |
| [TUI theme](tui/THEME.md) | Colour tokens, grid and drawing rules. |
| [Performance measurement](development/PERFORMANCE.md) | Local probes, transport latency, context reuse and recorded results. |
