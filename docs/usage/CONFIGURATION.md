# Configuration

Jecode keeps one personal configuration for your user account. There is no
project-level configuration file; project rules belong in
[`AGENTS.md`](PROJECT_INSTRUCTIONS.md). Connecting providers is covered in
[Providers](PROVIDERS.md).

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
| **Default model** | Provider, model and effort for new conversations. |
| **Default effort** | Reasoning effort for the default model. |
| **Default routing** | OpenRouter endpoint choice for the default model. |
| **Default permissions** | The [permission level](PERMISSIONS.md) new conversations start at. |
| **Auto-compact at** | When long conversations are compacted; applies immediately, including to the current conversation. |
| **Providers** | API keys and account sign-in; see [Providers](PROVIDERS.md). |

In `jecode --plain`, `/settings` runs the guided setup instead of a menu.

## Models and effort

| Action | Effect |
| --- | --- |
| `/model` | Choose a provider (when several are configured), then a model and effort, for this conversation. |
| `/model ID` | Switch this conversation to model `ID` on the current provider, without a catalog lookup. |
| `/effort` or `/effort NAME` | Change this conversation's reasoning effort. |
| `jecode --model ID "task"` | Use model `ID` for one launch without changing defaults. A different model starts at `default` effort. |

The effort selector lists the levels the model's catalog reports. `default`
leaves the choice to the provider. `/effort NAME` can request a level the
catalog does not list; the provider may reject it.

Switching model or provider keeps the conversation, its history, attachments,
processes and browser. Provider-specific data, such as encrypted reasoning, is
sent only back to the backend, account and model that produced it; another
model receives the portable history.

## Automatic compaction

Long conversations eventually exceed the model's context window. Before each
model request, Jecode checks whether the request would exceed a share of the
window, set by **Auto-compact at** (default 80%). When it would, Jecode first
shortens large tool results and then, if needed, asks the model to summarize
earlier work. The original history stays saved and readable. See
[Long-running work](LONG_WORK.md#context-and-compaction) for what is kept.

Set the threshold in `/settings` → **Auto-compact at** as a whole number from 1
to 99, with or without `%`. A saved change applies from the next model request,
including in the current conversation and its collaborators. A higher value
uses more of the window before compacting but leaves less room for the reply.

If even the instructions, your original requests and a minimal summary cannot
fit, the turn stops with an error that names the budget. Raise the threshold or
choose a model with a larger context window, then continue.

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
  "openrouter": {
    "api_key": "YOUR_OPENROUTER_KEY",
    "model": "provider/model-id",
    "effort": "default",
    "routing": { "provider/model-id": { "mode": "auto" } }
  },
  "context": { "auto_compact_percent": 80 },
  "permissions": { "default": "workspace" }
}
```

| Field | Meaning |
| --- | --- |
| `selection` | The default `backend` (`openrouter`, `openai` or `anthropic`), `model` and `effort` for new conversations. |
| `backends.<id>.model` | The last saved model for each provider, used when Jecode falls back to that provider. |
| `backends.openrouter.api_key` | The OpenRouter key, in plain text. Account tokens are never stored here. |
| `backends.<id>.routing` | Saved routing per model ID. |
| `openrouter` | The original OpenRouter block, still read and updated for compatibility. When present, its key, model, effort and routing take precedence for OpenRouter. |
| `context.auto_compact_percent` | Compaction threshold, 1–99. Default 80. |
| `permissions.default` | `read`, `workspace` or `full`. Default `workspace`. |

To pin an endpoint by hand, set the model's routing entry, in the `openrouter`
block when the file has one:

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
| `config.json` | Settings and the OpenRouter key. |
| `accounts/openai/`, `accounts/anthropic/` | ChatGPT and Claude account records and tokens. |
| `skills/` | Your [personal skills](SKILLS.md). |
| `sessions/<project>/` | [Saved conversations](SESSIONS.md), their complete command output, [attachments](ATTACHMENTS.md) and collaborator data, grouped by project directory. |
| `tmp/<project>/<session>/` | Each conversation's [temporary files](TEMPORARY.md). |
| `projects/<project>/map.json` | Notes saved by the project map tool; see [Tools](TOOLS.md#project-map). |
| `checkpoints/<project>-<id>/` | [Checkpoints](CHECKPOINTS.md), each expiring after 24 hours. |
| `browser/profiles/` | Named persistent browser profiles. |
| `cache/native/` | Native helpers extracted from the executable on first use. This directory must be writable. |

All of these files are local and can contain sensitive data: conversations,
command output and checkpoints hold project file contents, including files Git
ignores such as `.env`.

## Environment variables

| Variable | Effect |
| --- | --- |
| `JECODE_HOME` | Absolute path of the Jecode home. |
| `JECODE_BROWSER` | Path to the browser executable for the `browser` tool. |
| `OPENROUTER_API_KEY`, `OPENROUTER_MODEL` | Configure a run when no `config.json` exists; see [Providers](PROVIDERS.md#environment-variables). |

## Network proxies

HTTP proxy settings follow the platform:

- **Windows** uses the system proxy configuration (WinHTTP automatic proxy).
- **macOS** uses the system proxy settings.
- **Linux** reads `HTTPS_PROXY`, `HTTP_PROXY` and `NO_PROXY` (or their lowercase
  forms). Only `http://` proxies without credentials are supported; a proxy URL
  with another scheme makes the request fail rather than bypass the proxy.

`HTTP_PROXY` and `HTTPS_PROXY` have no effect on Windows and macOS. Loopback
addresses never use a proxy.
