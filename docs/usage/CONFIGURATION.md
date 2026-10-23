# Personal configuration

Start Jecode from the project directory. On the first interactive launch, setup
asks for a backend API key and model; `jecode setup` runs it again. OpenRouter is
the currently available backend. In the TUI, use `/settings` to change saved
defaults and the key. The key and defaults are
stored in plain text in `~/.jecode/config.json` (on Windows,
`%USERPROFILE%\.jecode\config.json`). Keep this personal file private.

## Models and effort

| Action | Scope |
| --- | --- |
| `/model [ID]` | Choose a model and then its effort for this conversation. |
| `/backend` or `/backend ID MODEL` | List available backends or switch this conversation to a configured backend and model. Plain chat also accepts `/backend ID` and prompts for a model. |
| `/effort [NAME]` | Change this conversation's reasoning effort. |
| `/settings` → **Default model** | Save the model and effort for future conversations and launches. |
| `/settings` → **Default effort** | Change the saved model's default effort. |
| `jecode --model provider/model-id "task"` | Override the model for one launch. |

The effort selector shows the levels supported by the model. A direct model ID
is accepted without a catalog lookup. `default` leaves
reasoning effort to the provider. Model and effort changes preserve the conversation;
saved defaults do not switch an existing conversation. Replace `provider/model-id`
with the model's catalog ID.

## Stored settings

The relevant fields are:

```json
{
  "selection": {
    "backend": "openrouter",
    "model": "provider/model-id",
    "effort": "default"
  },
  "backends": {
    "openrouter": {
      "api_key": "your-key",
      "routing": {
        "provider/model-id": { "mode": "auto" }
      }
    }
  },
  "openrouter": {
    "api_key": "your-key",
    "model": "provider/model-id",
    "effort": "default",
    "routing": {
      "provider/model-id": { "mode": "auto" }
    }
  },
  "context": { "auto_compact_percent": 80 }
}
```

`selection` is the default for new conversations; switching a conversation does
not change it. Backend credentials live under `backends.<id>`. The original
`openrouter` block remains readable and is updated for compatibility. Its key,
model, effort and routing take precedence for OpenRouter when both formats are
present, so edits made by an older installed binary remain effective. Its
values do not replace another backend's selection. Routing
choices belong to a backend, so an OpenRouter pin cannot affect another backend
using the same model ID. Other backend selections do not require
an OpenRouter key. The model ID and key must be valid; `effort` defaults to `default` in older
files. The file must be a JSON object of at most 64 KiB. Jecode retains unknown
fields when saving. An invalid or unreadable file blocks setup and launch with
an error; Jecode leaves it unchanged so you can correct it. Saves replace the
file atomically. New Unix configuration directories and files use permissions
`0700` and `0600`; Windows inherits permissions from the user's directory.
Jecode does not load `.env` files. See [Sessions](SESSIONS.md) for conversation
storage and resume. Per-call checkpoints have a fixed 24-hour lifetime and no
policy file; see [Checkpoints](CHECKPOINTS.md).

## Personal storage layout

Jecode uses `~/.jecode/` as its personal home (`%USERPROFILE%\.jecode\` on
Windows). `JECODE_HOME` can select another absolute directory. Each part has
its own purpose:

| Path | Contents and owner |
| --- | --- |
| `config.json` | Your key and saved settings; Jecode updates it through setup and settings. |
| `skills/` | Your reusable skill files; Jecode reads this library. |
| `sessions/` | Saved conversations and attachments, managed by conversation saves and session commands. |
| `tmp/` | Session working files, managed by Jecode's temporary-file actions. |
| `browser/profiles/` | Named persistent Chromium profiles selected by the `browser` tool. |
| `cache/` | Native helper binaries generated when needed. |
| `checkpoints/<workspace>-<id>/` | Jecode-managed `rc-ID.rcp` packs and `.internal/` identity and lease metadata. |

Normal configured launches create the checkpoint directory for the current
workspace, even before a modifying tool call; this does not scan or capture
workspace files. Provider-free checkpoint commands also open the workspace
directory. Each pack expires 24 hours after capture and Jecode removes expired
packs while running or on the next use. The deadline does not apply to
`.internal/` metadata. Checkpoint cleanup does not manage the other home paths.

## Automatic compaction threshold

`context.auto_compact_percent` is an integer from 1 to 99, defaulting to 80.
The threshold is a share of the effective model context window: the catalog
window, reduced if a provider context rejection establishes a lower ceiling.
It governs when Jecode reduces the next model request and how much input the
reduced request may use. Output room is calculated separately; raising the
threshold can leave less space for the reply.

Change it with `/settings` → **Auto-compact at**. The selector accepts a whole
number with an optional `%`. A successful save applies on the next model
request, including in the current conversation. Invalid input or a failed save
keeps the previous value. If even the system instructions, original user
requests and a useful summary cannot fit, the turn stops with a resource error;
raise the threshold or choose a model with more context. See
[Long-running work](LONG_WORK.md#active-context-and-original-history).

## Endpoint routing

Use `/routing` to choose how the current conversation reaches a model's
OpenRouter endpoints. The choice applies from the next request, including
summary requests. `/settings` → **Default routing** saves a choice for the
default model; saved choices apply to new conversations, model switches, single
prompts and piped tasks. A saved change does not switch the current
conversation. Models without an entry use Auto. In `--plain` chat, `/routing`
offers numbered choices; save defaults in the TUI or edit the JSON file.

| Choice | Behavior |
| --- | --- |
| **Auto — Best balance** | OpenRouter chooses endpoints with fallbacks. Jecode can add soft speed preferences from recent endpoint statistics. |
| **Specific endpoint** | Pin one listed endpoint without fallbacks. An absent, stale or incompatible pin stops the turn. |

Auto sends the conversation ID to help cache reuse. Its speed preferences do
not exclude endpoints; see [provider transport](../development/ARCHITECTURE.md#provider-transport)
for the selection method. Pin an endpoint when you need a particular provider
or variant and accept that its unavailability stops work.

The selector fetches the model's endpoint listing when opened; listings guide
turns for up to 10 minutes. A failed fetch leaves Auto using OpenRouter's
native routing, but an unverified pin stops with instructions to use `/routing`.
Automatic fetch retries wait at least a minute. If a pinned endpoint disappears,
choose Auto or another listed endpoint. A pin to an available endpoint can still
fail when that endpoint rejects or cannot serve a request.

Endpoint rows show listed input/output rates per million tokens and, when
available, recent throughput and first-token latency. `?` or `no stats` means
data is unavailable. The completion notice and `/routing` show the serving
provider and reported charge when OpenRouter supplies them; rates in the
selector are not an estimate of a turn's cost.

To set a per-model default manually, use:

```json
{
  "openrouter": {
    "routing": {
      "provider/model-id": { "mode": "endpoint", "endpoint": "provider/variant" },
      "provider/other-model": { "mode": "auto" }
    }
  }
}
```

These are fields within the full configuration, not a replacement for its key
and model. Unknown routing entries are retained but ignored; a `routing` value
that is not an object makes the file invalid. Conversations saved before
routing existed retain their original no-preference behavior until routing is
chosen. Account-level OpenRouter provider preferences can still influence
requests when Jecode has not overridden them.

## Precedence and environment

The personal file wins over `OPENROUTER_API_KEY` and `OPENROUTER_MODEL`. With no
file, those variables can configure a run without saving credentials; both are
required, though `--model` can supply the model. `--model MODEL` overrides the
model for one launch without editing defaults; a different model starts at
`default` effort and uses that model's saved routing choice. `jecode setup`
always manages the personal file.

`JECODE_HOME` selects an absolute configuration directory, including sessions,
temporary storage, checkpoint archives and the personal [skill library](SKILLS.md), and `JECODE_BASH`
selects the Bash executable. These are
useful for isolated development.

`/backend`, `/model`, `/effort` and `/routing` change the conversation without changing
saved defaults. Changing the key in `/settings` updates current authentication.
For what resumes with a conversation, see [Sessions](SESSIONS.md).
