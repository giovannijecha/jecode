# Files Jecode keeps

## Jecode home

Everything personal lives under the Jecode home, `~/.jecode/`
(`%USERPROFILE%\.jecode\` on Windows). Set `JECODE_HOME` to an absolute path to
use another directory, for example for an isolated test; a relative path is an
error.

## Stored files

| Path | Contents |
| --- | --- |
| `config.json` | Settings and saved provider API keys. |
| `mcp.json`, `mcp.lock` | Global [MCP connections](../usage/MCP.md), credentials and the shared configuration lock. |
| `lsp.json`, `lsp.lock` | Global [language servers](../usage/LSP.md), environment values and their configuration lock. |
| `accounts/openai/`, `accounts/anthropic/` | ChatGPT and Claude account records and tokens. |
| `skills/` | Your [personal skills](../usage/SKILLS.md). |
| `sessions/<project>/` | [Saved conversations](../usage/SESSIONS.md), grouped by project directory, with the project's shared [attachments](../usage/ATTACHMENTS.md) in `attachments/`. |
| `sessions/<project>/<session>/` | One conversation: its journal `session.jsonl`, lock `session.lock` and listing cache `summary.json`, with recovery copies beside them; its complete command output in `outputs/`, its [temporary files](../usage/TEMPORARY.md) in `tmp/` and its [collaborators](../usage/COLLABORATION.md) in `collaboration/`. |
| `browser/profiles/` | Named persistent browser profiles. |
| `cache/native/` | Native helpers extracted from the executable on first use. This directory must be writable. |

## Locks

The `mcp.lock`, `lsp.lock` and `session.lock` files coordinate access by
Jecode processes. Account updates and token refresh use `accounts/openai/.lock`
and `accounts/anthropic/.lock`; see the
[OpenAI](../development/OPENAI.md#sign-in-with-chatgpt) and
[Anthropic](../development/ANTHROPIC.md#account-lifecycle) adapters. Each
persistent browser profile holds a `.jecode-lease` lock, so one process uses
it at a time.

Empty file-modification locks live in the operating system's temporary
directory, separately for each account: `jecode-file-locks-<uid>` on Unix and
`jecode-file-locks` in the account's temporary directory on Windows. Up to 256
lock files contain no project data and coordinate processes using different
`JECODE_HOME` directories.

For credential storage, sensitive data and what leaves your machine, see
[Security](../../SECURITY.md).

## Installation files

Installations live outside the Jecode home, and `jecode uninstall` never
removes the home. A standalone installation from `INSTALL.ps1` or `INSTALL.sh`
is a root folder (by default `%LOCALAPPDATA%\Jecode` on Windows or
`${XDG_DATA_HOME:-$HOME/.local/share}/jecode` on Linux and macOS) holding:

| Path | Contents |
| --- | --- |
| `.jecode-install` | The marker that identifies the installation as Jecode's. |
| `bin/jecode`, `bin\jecode.exe` | The executable. |
| `bin/.jecode-update-*`, `bin/.jecode-previous-*` | A staged update, or a previous executable still running on Windows. `jecode update` and `jecode uninstall` remove them. |

npm and Volta installations are package folders their package manager owns;
see [Installation](../usage/INSTALLATION.md#update-jecode).
