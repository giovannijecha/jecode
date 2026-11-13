# Jecode documentation

Start with the [project README](../README.md) for installation and a first task.
Run Jecode commands from your project directory; run Cargo commands and repository
scripts from the Jecode repository root.

For project policies, see [Contributing](../CONTRIBUTING.md),
[Security](../SECURITY.md) and the [Code of conduct](../CODE_OF_CONDUCT.md).
Changes between versions are in the [release notes](../CHANGELOG.md).

## Get started

User guides in `usage/`:

| Guide | Covers |
| --- | --- |
| [Installation](usage/INSTALLATION.md) | Installing, building, updating and uninstalling. |
| [Platform support](usage/PLATFORMS.md) | System requirements, verification status and platform limitations. |
| [Providers](usage/PROVIDERS.md) | API keys, ChatGPT sign-in and the Claude account compatibility route. |
| [Terminal interface](usage/INTERFACE.md) | Sending, queueing, stopping, reading and quitting. |
| [Command line](usage/CLI.md) | Single prompts, pipes, run options, management commands, JSON output and exit codes. |

## Work with Jecode

Task guides in `usage/`:

| Guide | Covers |
| --- | --- |
| [Permissions](usage/PERMISSIONS.md) | Read, Workspace and Full; approvals; platform limits. |
| [Project instructions](usage/PROJECT_INSTRUCTIONS.md) | `AGENTS.md` files and `/init`. |
| [Personal skills](usage/SKILLS.md) | Reusable procedures in `~/.jecode/skills`. |
| [Tools](usage/TOOLS.md) | Files, search, commands, processes, web, browser and MCP. |
| [Tool activity](usage/TOOL_ACTIVITY.md) | Reading tool cards, Work chapters, diffs and Ctrl+O. |
| [MCP connections](usage/MCP.md) | Global HTTP connections, tokens, browser sign-in and remote tool permissions. |
| [Language servers](usage/LSP.md) | Enabled local servers, semantic navigation, diagnostics and execution scope. |
| [Saved conversations](usage/SESSIONS.md) | Resuming, recovering and deleting conversations. |
| [Attachments](usage/ATTACHMENTS.md) | Files and clipboard images. |
| [Collaborating agents](usage/COLLABORATION.md) | Delegated assignments, isolated workspaces and review. |
| [Questions and plans](usage/QUESTIONS_AND_PLANS.md) | Answering questions and following the agent's plan. |
| [Copy and export](usage/COPY_AND_EXPORT.md) | Copying answers and saving conversations as JSON. |
| [Temporary files](usage/TEMPORARY.md) | Each conversation's working area and its cleanup. |
| [Long-running work](usage/LONG_WORK.md) | Retries, the context meter, compaction summaries and original history. |

## Reference

Lookup pages in `reference/`:

| Reference | Contents |
| --- | --- |
| [Configuration](reference/CONFIGURATION.md) | Settings, `config.json`, routing, environment variables and proxies. |
| [Files Jecode keeps](reference/FILES.md) | The Jecode home, `JECODE_HOME`, local directories and locks. |
| [Commands and keys](reference/COMMANDS.md) | Slash commands, keyboard controls and `jecode` shell commands. |
| [Tool reference](reference/TOOL_REFERENCE.md) | Exact tool arguments, defaults and limits. |
| [Command language](reference/SHELL.md) | Syntax and built-ins of the `bash` tool. |
| [Markdown display](reference/MARKDOWN.md) | How answers are formatted in the terminal. |

## Development

Contributor procedures and implementation details in `development/`:

| Reference | Contents |
| --- | --- |
| [Architecture](development/ARCHITECTURE.md) | Modules, request lifecycle, storage and native boundaries. |
| [Development and testing](development/TESTING.md) | Toolchain, checks, fixtures, terminal-host checklist and CI. |
| [TUI theme](development/TUI_THEME.md) | Colour tokens, grid and drawing rules. |
| [Performance measurement](development/PERFORMANCE.md) | Measurement rules and reproducible probes. |
| [Context checks](development/CONTEXT_CHECKS.md) | Offline replay and the limits of context screening. |
| [OpenAI backend](development/OPENAI.md) | API-key and ChatGPT account authentication and Responses translation. |
| [Anthropic backend](development/ANTHROPIC.md) | API-key and Claude account authentication and Messages translation. |
| [OpenRouter backend](development/OPENROUTER.md) | Catalog, endpoint routing, streaming, caching and verification. |

## Maintainers

Operations in `maintainers/` that require maintainer accounts or permissions:

| Runbook | Contents |
| --- | --- |
| [Releasing](maintainers/RELEASING.md) | Preparing, verifying and publishing native and npm release artifacts. |
| [GitHub MCP App](maintainers/GITHUB_MCP_APP.md) | App registration, device sign-in and the public manifest. |
| [Vercel MCP client](maintainers/VERCEL_MCP_CLIENT.md) | Public OAuth metadata, hosting and client setup. |

## Reports

Dated evidence in `reports/`; results apply to the builds and hosts recorded there:

| Report | Contents |
| --- | --- |
| [Report index and rules](reports/README.md) | Metadata, snapshot policy and how procedures link to evidence. |
| [Agent benchmarks, November 2026](reports/AGENT_BENCHMARK_2026-11.md) | Historical comparisons, methods, quality misses and per-run CSV. |
| [Context screening, November 2026](reports/CONTEXT_SCREENING_2026-11.md) | Offline replay, live screens and review checks on earlier builds. |
| [Historical performance results](reports/PERFORMANCE_RESULTS.md) | Dated transport, UI, parsing, file-edit and storage measurements. |
| [MCP OAuth checks, 2026-11-03](reports/MCP_OAUTH_2026-11-03.md) | Recorded authorization, refresh and discovery results. |
