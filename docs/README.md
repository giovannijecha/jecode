# Documentation

Start with the [project README](../README.md) for installation and first launch.
Runtime commands run from your project directory. Cargo commands and repository
scripts run from the Jecode repository root.

## Usage

| Task | Guide |
| --- | --- |
| Choose a model, reasoning effort or endpoint; edit defaults | [Configuration](usage/CONFIGURATION.md) |
| Supply project rules with `JECODE.md` | [Project instructions](usage/PROJECT_INSTRUCTIONS.md) |
| Understand file tools, command output and background processes | [Tools](usage/TOOLS.md) |
| Resume, recover or delete a conversation | [Saved sessions](usage/SESSIONS.md) |
| Add files and clipboard images | [Attachments](usage/ATTACHMENTS.md) |
| Copy an answer or code block | [Copy](usage/COPY.md) |
| Save a conversation snapshot | [Export](usage/EXPORT.md) |
| Inspect or clear session working files | [Temporary files](usage/TEMPORARY.md) |
| Understand retries, compaction and retained history | [Long-running work](usage/LONG_WORK.md) |

## Terminal interface

| Reference | Contents |
| --- | --- |
| [Composer and controls](tui/COMPOSER.md) | Editing, keyboard shortcuts, commands, selectors and pending messages. |
| [Markdown](tui/MARKDOWN.md) | Supported syntax, code blocks, tables and display limits. |
| [Scrolling and resize](tui/RESIZE.md) | Reading position, reflow and terminal restoration. |
| [Theme and tool cards](tui/THEME.md) | Semantic colors, components and tool presentation. |

## Development

| Reference | Contents |
| --- | --- |
| [Architecture](development/ARCHITECTURE.md) | Module map, agent flow, persistence and native boundaries. |
| [Development and testing](development/TESTING.md) | Offline checks, isolated fixtures and manual smoke tests. |
| [Platform verification](development/PLATFORMS.md) | Native test commands, local coverage and host checks. |
| [Performance](development/PERFORMANCE.md) | Recorded measurements, methodology and current benchmark commands. |
