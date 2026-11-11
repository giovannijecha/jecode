# Configuration

Jecode keeps one personal configuration for your user account. There is no
project-level configuration file; project rules belong in
[`AGENTS.md`](../usage/PROJECT_INSTRUCTIONS.md). Connecting providers is covered in
[Providers](../usage/PROVIDERS.md).

## Change settings

Most settings have two scopes:

- **This conversation.** `/model`, `/effort`, `/routing` and `/permissions`
  change the current conversation. The choice is saved with it and restored on
  resume, but new conversations do not inherit it.
- **Defaults.** `/settings` saves defaults for new conversations, launches,
  single prompts and pipes. Saving a default does not change the current
  conversation, except where noted below.

| `/settings` item | What it sets |
| --- | --- |
| **Default model** | API keys or Credentials, then provider, model and effort for new conversations. |
| **Default effort** | Reasoning effort for the default model. |
| **Default routing** | OpenRouter endpoint choice for the default model. |
| **Default permissions** | The [permission level](../usage/PERMISSIONS.md) new conversations start at. |
| **Context** | **Auto-compact at** and **Prompt cache**; both apply immediately, including to the current conversation. |
| **Theme** | Which inks the interface uses: **Auto**, **Dark** or **Light**. Applies at once, to the current conversation and to new ones; see [Theme](#theme). |
| **Providers** | API keys and account sign-in; see [Providers](../usage/PROVIDERS.md). |
| **MCP** | Global remote connections; see [MCP](../usage/MCP.md). |
| **LSP** | Global, explicitly enabled local language servers; see [Language servers](../usage/LSP.md). |

In `jecode --plain`, `/settings` runs the guided setup instead of a menu.

## Models and effort

| Action | Effect |
| --- | --- |
| `/model` | Choose API keys or Credentials and a configured provider (when several are available), then a model and effort, for this conversation. |
| `/model ID` | Switch this conversation to model `ID` on the current provider. The TUI opens the effort selector when the catalog lists `ID`. An unlisted ID, a failed catalog load or `--plain` applies it at `default` effort. |
| `/effort` or `/effort NAME` | Change this conversation's reasoning effort. |
| `jecode --model ID "task"` | Use model `ID` for one launch without changing defaults. A different model starts at `default` effort. |

The effort selector uses the model's catalog or documented API capabilities.
For known OpenAI API models, Jecode supplies the levels documented by OpenAI
because its API catalog omits them. Unknown model IDs retain `default`, which
leaves the choice to the provider. On the OpenAI API and ChatGPT routes,
`/effort NAME` can request a level the selector does not list; the provider may
reject it. On OpenRouter and Anthropic, Jecode refuses a level the model's catalog
does not list, or a model the catalog does not contain.

Switching model or provider keeps the conversation, its history, attachments,
processes and browser. Provider-specific data, such as encrypted reasoning, is
sent only back to the backend, account and model that produced it; another
model receives the portable history.

## Automatic compaction

Requests grow with the conversation. Until a request would exceed a share of the
model's context window, set by **Auto-compact at** (default 80%), Jecode only
appends to it, so earlier messages can come from the provider's prompt cache.
At that point the model summarizes earlier work and keeps the recent messages
verbatim. The original history stays saved and readable. See
[Long-running work](../usage/LONG_WORK.md#context-and-compaction) for what is kept.

Set the threshold in `/settings` → **Context** → **Auto-compact at** as a whole
number from 1 to 99, with or without `%`. A saved change applies from the next model request,
including in the current conversation and its collaborators. A higher value
uses more of the window before compacting but leaves less room for the reply.

If even the instructions, your original requests and a minimal summary cannot
fit, the turn stops with an error that names the budget. Raise the threshold or
choose a model with a larger context window, then continue.

## Prompt cache

`/settings` → **Context** → **Prompt cache** chooses **5 minutes**, the default,
or **1 hour** for the unchanged start of Claude requests. The choice applies
from the next request, in the current conversation and its collaborators, for
Claude on Anthropic, the Claude plan and OpenRouter. Jecode always requests
one-hour retention for its own shared instructions and tools on Anthropic and
the Claude plan. Other models keep their provider's automatic caching.

These settings request a cache lifetime; the provider determines whether a
prefix is cached or reused.

## Theme

Jecode never paints a background; the terminal keeps its own. `/settings` → **Theme**
chooses the inks the text and surfaces use:

| Choice | Inks |
| --- | --- |
| **Auto** | Follows the terminal's reported background: light inks on a light terminal, dark inks otherwise and when the terminal does not answer. This is the default. |
| **Dark** | Inks for a dark terminal, whatever the terminal says. |
| **Light** | Inks for a light terminal, whatever the terminal says. |

A choice that does not match the terminal reads badly, because no background is
drawn to make up for it. The change applies at once, to the current conversation
and to new ones. See [TUI theme](../development/TUI_THEME.md) for how the terminal is
queried.

## OpenRouter endpoint routing

A model on OpenRouter is often served by several endpoints. `/routing` chooses
how the current conversation reaches them; `/settings` → **Default routing**
saves a choice for the default model. Other providers have no routing choice.

| Choice | Behavior |
| --- | --- |
| **Auto — Best balance** | OpenRouter picks endpoints and falls back between them. When recent statistics are available, Jecode adds soft preferences for faster endpoints without excluding any. |
| A specific endpoint | Requests go only to that endpoint, without fallbacks. If it disappears, becomes stale or is incompatible, the turn stops and asks you to choose again in `/routing`. |

The selector lists each endpoint's input and output price per million tokens
and, when available, recent throughput and first-token latency; `?` or
`no stats` means OpenRouter did not report them. Prices are list rates, not an
estimate of a turn's cost. After a turn, the status row and `/routing` show the
provider that served it and, when OpenRouter reports it, the charge.

Endpoint listings are fetched when the selector opens and reused for up to 10
minutes. If a fetch fails, Auto keeps working with OpenRouter's own routing, but
a pinned endpoint cannot be verified and the turn stops. Account-level provider
preferences in your OpenRouter settings still apply where Jecode does not
override them. In `--plain`, `/routing` offers numbered choices for the
conversation; save defaults in the TUI or in the file.

## The configuration file

Settings live in `~/.jecode/config.json` (`%USERPROFILE%\.jecode\config.json` on
Windows). Jecode writes it from setup and `/settings`; you can also edit it.
Files containing the retired root-level `openrouter` block are unsupported and
left unchanged. Keep provider settings under `backends` and remove the retired
block manually before launching; Jecode does not migrate it.
A typical file after connecting OpenRouter and then choosing a ChatGPT model as
the default:

```json
{
  "selection": {
    "backend": "openai",
    "model": "chatgpt-model-id",
    "effort": "default"
  },
  "backends": {
    "openai": { "model": "chatgpt-model-id" },
    "openrouter": {
      "api_key": "YOUR_OPENROUTER_KEY",
      "model": "provider/model-id",
      "routing": { "provider/model-id": { "mode": "auto" } }
    }
  },
  "context": { "auto_compact_percent": 80, "prompt_cache": "5m" },
  "appearance": { "theme": "auto" },
  "permissions": { "default": "workspace" }
}
```

| Field | Meaning |
| --- | --- |
| `selection` | The default `backend`, `model` and `effort` for new conversations. API-key IDs are `openrouter`, `openai-api` and `anthropic-api`; subscription IDs remain `openai` and `anthropic`. |
| `backends.<id>.model` | The last saved model for each provider, used when Jecode falls back to that provider. |
| `backends.<id>.api_key` | The API key for `openrouter`, `openai-api` or `anthropic-api`, in plain text. Account tokens are never stored here. |
| `backends.<id>.routing` | Saved routing per model ID. |
| `context.auto_compact_percent` | Compaction threshold, 1–99. Default 80. |
| `context.prompt_cache` | How long Claude prompt caches last: `5m` or `1h`. Default `5m`. |
| `appearance.theme` | `auto`, `dark` or `light`. Default `auto`. |
| `permissions.default` | `read`, `workspace` or `full`. Default `workspace`. |

To pin an endpoint by hand, set the model's entry under
`backends.openrouter.routing`:

```json
{ "mode": "endpoint", "endpoint": "provider/variant" }
```

Rules for the file:

- It must be a JSON object of at most 64 KiB.
- Unknown fields are kept when Jecode saves. Unknown routing entries are ignored.
- If the file is invalid or unreadable, setup and launch stop with an error and
  leave the file unchanged so you can fix it.
- Saves replace the file atomically. On Unix a new file gets mode `0600` and a
  new directory `0700`; on Windows the file inherits your profile's permissions.
- Jecode never reads `.env` files.

## Files Jecode keeps

Everything personal lives under the Jecode home, `~/.jecode/`
(`%USERPROFILE%\.jecode\` on Windows). Set `JECODE_HOME` to an absolute path to
use another directory, for example for an isolated test; a relative path is an
error.

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

Empty file-modification locks live in the operating system's temporary
directory, separately for each account: `jecode-file-locks-<uid>` on Unix and
`jecode-file-locks` in the account's temporary directory on Windows. The 256
lock files contain no project data and coordinate processes using different
`JECODE_HOME` directories.

All of these files are local and can contain sensitive data: conversations
and command output hold project file contents, including files Git ignores such
as `.env`.

## Environment variables

| Variable | Effect |
| --- | --- |
| `JECODE_HOME` | Absolute path of the Jecode home. |
| `JECODE_BROWSER` | Path to the browser executable for the `browser` tool. |
| `OPENROUTER_API_KEY`, `OPENROUTER_MODEL` | Configure a run when no `config.json` exists; see [Providers](../usage/PROVIDERS.md#environment-variables). |

## Network proxies

HTTP proxy settings follow the platform:

- **Windows** uses the system proxy configuration (WinHTTP automatic proxy).
- **macOS** uses the system proxy settings.
- **Linux** reads `HTTPS_PROXY`, `HTTP_PROXY` and `NO_PROXY` (or their lowercase
  forms). Only `http://` proxies without credentials are supported; a proxy URL
  with another scheme makes the request fail rather than bypass the proxy.

`HTTP_PROXY` and `HTTPS_PROXY` have no effect on Windows and macOS. Loopback
addresses never use a proxy.

OpenAI conversations first try a WebSocket to the same host, using the same proxy
route. If the connection is refused, does not open within 15 seconds or keeps
closing, the request goes over HTTP and that conversation stays on HTTP for 5 minutes.
