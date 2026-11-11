# OpenRouter backend

The `openrouter` adapter uses an API key and OpenRouter's chat-completions
route at `https://openrouter.ai/api/v1`. `backend/openrouter/` owns catalog and
endpoint metadata, request translation, streaming, routing and reported usage.
The agent loop, tools and
saved conversation are shared with the other backends. For key setup, see
[Providers](../usage/PROVIDERS.md#openrouter-api-key); for the user's endpoint
choices, see [Configuration](../reference/CONFIGURATION.md#openrouter-endpoint-routing).
For key storage and access permissions, see [Credentials](../../SECURITY.md#credentials).

## Catalog and capacity

`GET /models?supported_parameters=tools` supplies the selector catalog. Search
adds a percent-encoded `q` and `limit=10`. Jecode keeps distinct, valid IDs whose
entry declares `tools`; it reads display names, prompt and completion prices,
and declared reasoning efforts without guessing unsupported levels. Prices in
the catalog are converted to USD per million tokens. A model's exact catalog
entry supplies its context length, optional top-provider output limit and input
modalities. Missing context stays unknown; metadata for the selected model is
cached on its backend instance.

`GET /models/{author}/{slug}/endpoints` supplies endpoint tags, listed prices,
limits, capabilities and available 30-minute performance statistics. Invalid or
duplicate tags are skipped. Missing and invalid measurements remain unknown,
not zero. A pinned endpoint narrows a known model context limit; its reported
output limit takes precedence when present. Endpoint metadata is fresh for ten
minutes; a failed fetch waits one minute before another attempt.

## Request and routing

Each generation sends `POST /chat/completions` with `stream: true`, the selected
model, projected message history and function tool definitions when requested.
An explicit output allowance becomes `max_completion_tokens`; a non-default
effort becomes `reasoning.effort`. The adapter sends the API key as a Bearer
header and requests OpenRouter metadata for this route. Canonical tool calls
are translated to function-call messages; compatible native reasoning and
function-call data can be replayed to the same backend, account and model.

Attachments are materialized at the adapter boundary: images become
`image_url` parts and PDFs become `file` parts. Requests carrying files select
the `file-parser` plugin's `native` engine when the model declares file input,
or `cloudflare-ai` otherwise.

Auto leaves provider selection and fallback to OpenRouter. With enough fresh,
comparable endpoint measurements, it adds soft throughput and first-token
latency preferences; otherwise it sends no preference. Auto and Default
routing modes send a conversation `session_id` for cache affinity. An
endpoint pin instead sends one `provider.order` entry with
`allow_fallbacks: false` and excludes known sub-variants of that tag. Before a
turn, a pin needs a fresh listing that contains a compatible endpoint; known
lack of tools or required reasoning blocks it. A failed pinned request explains
how to choose Auto or another endpoint with `/routing`.

## Streaming and cache

The stream decoder handles fragmented SSE, emits text, reasoning, tool-work
and usage updates, and assembles function arguments by index. It returns
executable calls only after `[DONE]`, contiguous call indexes and a complete
`stop` or `tool_calls` finish reason. A JSON completion on a non-SSE response
uses the same final validation. Reported serving provider, token counts,
cached and written tokens, and charge are retained when supplied; missing
cost is not treated as free.

Shared request projection anchors changed instructions and runtime state at
their first send, adding later changes instead of rewriting accepted history.
It adds a continuation note when a new request would otherwise end with an
assistant answer. OpenRouter removes internal state markers before sending
the request. For agent requests to Claude model IDs, it marks up to four recent
durable text boundaries with `cache_control`; the chosen prompt-cache lifetime
adds `ttl: "1h"` for the one-hour option. Known OpenAI GPT-5.6 and GPT-6 IDs
receive the plain ephemeral markers. The markers and stable `session_id` seek
cache reuse; the adapter reports a hit only from provider usage fields.

## Failures and verification

HTTP status and provider error codes distinguish context, temporary, rate-limit,
rejected-key and terminal failures. Retry-After is retained for the shared
retry path. Diagnostics redact the configured key before truncation. A
truncated stream, malformed tool calls or an incomplete finish cannot execute
calls from that response. The route does not switch credentials on an error.

After setting up the platform toolchain (on Windows, follow
[Windows toolchain](TESTING.md#windows-toolchain)), run the focused offline
fixtures from the repository root:

```text
cargo test --locked --offline backend::openrouter::
```

These tests exercise catalog and endpoint parsing, routing, cache markers,
native HTTP, SSE assembly, tool validation, usage and error handling with
loopback responses and synthetic credentials. A passing fixture run does not
verify current OpenRouter service behavior, a particular model, or live
inference. Live provider checks are opt-in; see
[Development and testing](TESTING.md#live-provider-checks).
