# Jecode documentation

New to Jecode? Start with the [project README](../README.md): it covers what
Jecode does, installation and a first task. Run Jecode commands from your
project directory; run Cargo commands and repository scripts from the Jecode
repository root.

## Get started

| Guide | Covers |
| --- | --- |
| [Installation](usage/INSTALLATION.md) | Requirements, building, installing, updating and uninstalling. |
| [Providers](usage/PROVIDERS.md) | Connecting OpenRouter, a ChatGPT account or a Claude account. |
| [Terminal interface](tui/INTERFACE.md) | Working in the fullscreen interface: sending, queueing, stopping, reading and quitting. |
| [Command line](usage/CLI.md) | Single prompts, pipes, plain chat, options and exit codes. |

## Work with Jecode

| Guide | Covers |
| --- | --- |
| [Permissions](usage/PERMISSIONS.md) | Read, Workspace and Full; approvals; platform limits. |
| [Project instructions](usage/PROJECT_INSTRUCTIONS.md) | `AGENTS.md` files and `/init`. |
| [Personal skills](usage/SKILLS.md) | Reusable procedures in `~/.jecode/skills`. |
| [Tools](usage/TOOLS.md) | What the agent can do: files, search, commands, processes, web and browser. |
| [Checkpoints](usage/CHECKPOINTS.md) | Recovering files after a modifying tool call. |
| [Saved conversations](usage/SESSIONS.md) | Resuming, recovering and deleting conversations. |
| [Attachments](usage/ATTACHMENTS.md) | Files and clipboard images. |
| [Collaborating agents](usage/COLLABORATION.md) | Delegated assignments, isolated workspaces and review. |
| [Questions and plans](usage/QUESTIONS_AND_PLANS.md) | Answering the agent's questions and following its plan. |
| [Copy and export](usage/COPY_AND_EXPORT.md) | Copying answers and saving conversations as JSON. |
| [Temporary files](usage/TEMPORARY.md) | Each conversation's working area and its cleanup. |
| [Long-running work](usage/LONG_WORK.md) | Retries, context compaction and history. |

## Reference

| Reference | Contents |
| --- | --- |
| [Configuration](usage/CONFIGURATION.md) | Settings, `config.json`, routing, stored files, environment variables and proxies. |
| [Commands and keys](tui/COMMANDS.md) | Every slash command and keyboard control in the TUI. |
| [Tool activity](tui/TOOL_ACTIVITY.md) | Reading tool cards, Work chapters, diffs and Ctrl+O. |
| [Markdown display](tui/MARKDOWN.md) | How answers are formatted in the terminal. |
| [Tool reference](usage/TOOL_REFERENCE.md) | Exact tool arguments, defaults and limits. |
| [Command language](usage/SHELL.md) | Syntax and built-ins of the `bash` tool. |

## Development

| Reference | Contents |
| --- | --- |
| [Development and testing](development/TESTING.md) | Toolchain, checks, native fixtures, live checks and CI. |
| [Architecture](development/ARCHITECTURE.md) | Modules, request lifecycle, storage and native boundaries. |
| [Platform support](development/PLATFORMS.md) | Verification status, platform limits and host checks. |
| [OpenAI backend](development/OPENAI.md) | Sign in with ChatGPT and the Responses route. |
| [Anthropic backend](development/ANTHROPIC.md) | The Claude account compatibility route. |
| [TUI theme](tui/THEME.md) | Colour tokens, grid and drawing rules. |
| [Performance measurement](development/PERFORMANCE.md) | Probes, method and recorded results. |
