# Personal configuration

Use `/settings` to update saved defaults or the OpenRouter key. `jecode setup`
opens guided configuration outside an active conversation.

## Stored settings

Settings live outside your project in `~/.jecode/config.json`. On Windows this is
`%USERPROFILE%\.jecode\config.json`. The format is:

```json
{
  "openrouter": {
    "api_key": "your-key",
    "model": "provider/model-id",
    "effort": "default"
  },
  "context": {
    "auto_compact_percent": 80
  }
}
```

Credentials are deliberately stored in plain text. Jecode does not load `.env`
files or print the key. Conversations autosave under `~/.jecode/sessions/`, with
the configured key masked. Configuration writes are atomic; unknown fields are
retained for future settings. Corrupt, incomplete, oversized
or read-only files are left unchanged and reported. Configuration is limited to
64 KiB. New Unix configuration directories/files use modes 0700/0600; Windows
uses the permissions inherited from the user's directory.

## Automatic compaction threshold

`context.auto_compact_percent` sets where automatic context reduction starts, as
a share of the effective context window: the model's catalog `context_length`,
lowered by any ceiling learned from provider context rejections. It accepts
whole numbers from 1 to 99 and defaults to 80; files written before this
setting load with 80. A value outside that range makes the file invalid, like
other malformed settings, and the file is left unchanged.

Change it in `/settings` under **Auto-compact at**. Enter a whole percentage,
optionally followed by `%`; an invalid entry is rejected and the previous value
stays active and saved. A successful save applies from the next model request of
the current conversation and to every new or resumed conversation; it is a user
preference, not saved per conversation. If the file cannot be written, the
previous value stays active. `jecode setup`, model and effort changes and key
replacement keep the saved value.

The input budget, this percentage of the effective window, both triggers
reduction and limits its result. The output allowance is calculated separately
from the space left in the effective window, so a higher threshold leaves less
room for the model's reply before reduction starts. A threshold too small for the system instructions, original
user requests and a useful summary stops the turn with a resource error naming
the budget; history and the previous context are preserved. See
[LONG_WORK.md](LONG_WORK.md#active-context-and-original-history).

## Endpoint routing

OpenRouter serves most models through several provider endpoints. `/routing`
chooses how the current conversation reaches them, from its next request,
summary and compaction requests included:

- **Auto — Best balance**, the default, keeps OpenRouter's native routing with
  load balancing and fallbacks. It never sorts by price, which would bypass
  OpenRouter's tool-calling quality routing (Auto Exacto). Requests carry the
  saved conversation's identifier as `session_id` to help reuse cached prompts.
  Auto Exacto and fallbacks can still move requests to another endpoint.
  When a fresh listing has at least three
  comparable measured endpoints, Auto also sends
  `provider.preferred_min_throughput` (half their median throughput) and
  `provider.preferred_max_latency` (twice their median time to first token).
  These move markedly slow endpoints behind the others; no endpoint is
  excluded and unmeasured endpoints never count as fast. The thresholds are
  provisional and kept together in `policy` in `src/openrouter/routing.rs`.
- **A specific endpoint** sends `provider.order` with its tag and
  `allow_fallbacks: false`, plus `provider.ignore` for listed tags that extend
  it, so requests reach only that endpoint. They fail while it is unavailable,
  and the error points to `/routing`. Jecode never adds `require_parameters`;
  a structured summary format the endpoint is listed without is left out
  instead. Endpoints listed without tool calls, or without reasoning required by
  the selected effort, cannot be chosen. Each turn verifies the pin against a
  fresh listing; if the listing is unavailable, expired or no longer contains
  that exact endpoint, the turn stops before sending a completion. This avoids
  treating an unverified provider prefix as an exact endpoint pin.

Endpoint rows come from OpenRouter's endpoint listing for the model. Listed
prices already include provider discounts and are shown once, as $/M input/output
tokens; speed and first-token latency are 30-minute medians for text generation.
Unknown values stay `?` or `no stats`, never zero. The listing is fetched when
the selector opens, or before a turn when the last one is over 10 minutes old.
A failed fetch leaves Auto to native routing; an unverified pin stops with a
message pointing to `/routing`. Automatic fetch retries wait a minute. Esc
cancels a pending fetch. Without a listing the selector still offers Auto and
the current choice.

After each turn the completion notice, and `/routing`, show the provider that
served it and what OpenRouter reported charging (`usage.cost`), with cached
input tokens and, for your own provider keys, the upstream charge. Jecode opts
into routing metadata to identify the selected provider when reported. Charges
reported by truncated or rejected completions are included too. Last-turn
charges are cleared when changing conversations. Listed prices are rates, not
charges; unavailable billing information is not estimated.

`/settings` → **Default routing** saves the routing for the default model:

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

Models without an entry use Auto. Saved routing applies to new conversations,
model switches and single prompts or piped tasks; it never changes the current
conversation, and a model switch never keeps another model's pin. Files written
before routing existed load unchanged. Entries this version cannot read are
skipped and kept in the file; a `routing` value that is not an object makes the
file invalid. `--plain` offers the same numbered choices for the conversation;
change saved defaults in the TUI or this file. Preferences saved in your
OpenRouter account, such as a default provider sort, can still apply to
requests; Jecode does not override them.

## Precedence and environment

The saved configuration takes precedence over legacy environment variables.
`--model` overrides the model for one run without changing saved defaults. A
changed model starts with its default effort; the saved pair applies otherwise.
Older configuration files without `effort` keep working and use `default`.
If no file exists, `OPENROUTER_API_KEY` and `OPENROUTER_MODEL` can still configure
a run without saving credentials to `config.json`; sessions still autosave.
`--model` may supply the environment-only model.
`jecode setup` always manages the personal file.

## Advanced overrides

- `JECODE_HOME`: absolute configuration directory, useful for isolated development.
- `JECODE_BASH`: path to the installed Bash executable.
- `INSTALL.ps1 -InstallRoot ABSOLUTE_PATH -NoPath`: install elsewhere without
  changing PATH, useful for verification.

## Conversation overrides

Model, effort and routing changes keep the conversation and do not overwrite saved
defaults. Settings apply to future `/new` conversations and launches; changing
the key also updates authentication for the current conversation, and
Auto-compact at also applies to the current conversation. Across a
model boundary, portable content and tool calls/results remain in context.
Older provider-specific reasoning fields stay in the export rather than being
sent to the new model. Within the same model they are retained for tool follow-up.

See [SESSIONS.md](SESSIONS.md) for persisted model/effort state and explicit resume.
