# OpenAI backend

The OpenAI adapter uses Sign in with ChatGPT alongside OpenRouter. It has no
OpenAI API-key path. Preserve the independent `Backend` and `Service` boundaries:
authentication, wire format, provider state and caching belong to the adapter;
tools, canonical history and terminal rendering remain shared.

## Sign in with ChatGPT

Use OpenAI's documented [registration and sign-in flow for open-source
apps](https://developers.openai.com/siwc/token-sharing-open-source/sign-in),
verified on October 24, 2026. This is a native public client with its own identity,
without a client secret or partner API key. It must not read Codex credentials,
reuse a Codex client ID, or depend on the Codex CLI or app-server.

- Persist a stable `ext_agent_host_id` for the Jecode host. Initial registration
  sends `client_id=dynamic_agent_client` and `agent_name_hint=Jecode`.
- Start the loopback listener before launching the system browser. Use
  `http://127.0.0.1:<port>/auth/callback`; keep the exact URI within each attempt.
  Later attempts may change only the port.
- Generate fresh random state, OIDC nonce and PKCE verifier for each attempt;
  use S256. Keep an attempt separate from the active account until validated.
- Initial registration returns an issued client ID. Exchange the code with that
  ID, not `dynamic_agent_client`, and retain it for the registered account.
  Returning callbacks must not replace it with an unexpected client ID.
- Validate state and OAuth errors before code exchange. Verify the ID token's
  signature against OpenAI JWKS, issuer, issued-client audience, expiry and nonce.
  Use the verified subject as account identity; returning sign-in must match the
  selected account.
- Request the documented identity and plan-use scopes with resource
  `https://api.openai.com/v1`. Require granted `chatgpt.tokens.use.direct` before
  plan inference; an ID token alone is insufficient.
- Keep protected, atomically written local records per issued client ID and
  verified identity. Replace rotating refresh tokens, access tokens, scopes and
  expiry together. Preserve account/workspace separation and the verified ID
  token in the local account record. Never log credentials.

The native crypto boundary uses Windows PowerShell/.NET for secure random
bytes, SHA-256 and RSA signature verification. Unix requires the installed
`openssl` executable. HTTPS uses system curl 8.3+ with certificate verification;
token request bodies travel through stdin and inference bearer tokens through
the curl child's environment. Cancellation and timeouts bound these operations.
No external Rust crate or copied cryptographic implementation is used.
Only `JECODE_OPENAI_ACCESS_TOKEN` is reserved for Jecode's OpenAI transport;
unrelated `OPENAI_API_KEY` and `ACCESS_TOKEN` variables remain available to tools.

Account records are plaintext JSON under `~/.jecode/accounts/openai/`, separately
from `config.json`. Unix uses `0700` directories and `0600` files; Windows applies an
ACL restricted to the current user. A stable host identity and each verified
registration survive logout. Refresh rereads the record under an OS file lock
and replaces the rotating token set atomically. Shared redaction sources keep
new access, refresh and ID tokens masked in existing consumers.

Account discovery skips malformed record content and reports a count at startup,
before fullscreen rendering. Valid accounts remain available; filesystem access
errors and a corrupt selected account still return explicit errors. Sign-out
removes local credentials under the account lock even when already cancelled,
then releases the lock before attempting remote revocation.

Invalid or unrelated loopback callbacks receive HTTP 400 without ending the
sign-in attempt. A matching provider error still ends it. During refresh, an
omitted `refresh_token` retains the previous token; a newly issued token replaces
it, following [RFC 6749 section 6](https://www.rfc-editor.org/rfc/rfc6749#section-6).
A null token is treated as omitted for compatibility.

CLI setup uses a scoped console reader for Enter-to-cancel during sign-in:
system `cmd.exe` on Windows and Bash on Unix. The reader process is stopped and
reaped, and its watcher joined, before setup reads another prompt, including
after completion or failure. This uses installed system tools without a Rust
runtime dependency.

The account picker separates registrations even when they share an email.
Conversation origins include the verified registration, so another account
receives portable history without the original account's opaque output items.

## Responses route

The [preview limitations](https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations)
define the ChatGPT plan route, rather than the full API-key Responses surface:

- List `GET https://api.openai.com/v1/models` with the selected account's token;
  keep `models` with `visibility: "list"`, server order, `display_name` and `slug`.
- Infer through `POST https://api.openai.com/v1/responses` with `stream: true`,
  `store: false` and the required history in `input`. Omit HTTP continuation IDs,
  server conversations, output-token caps and unsupported sampling parameters.
- Convert system instructions to `instructions`; translate local calls and
  results into function-call items. Group function definitions in a namespace.
- Preserve ordered native output items, encrypted reasoning and assistant phases.
  The [reasoning guide](https://developers.openai.com/api/docs/guides/reasoning#preserve-reasoning-without-stored-responses)
  documents automatic encrypted content with `store: false`; its legacy include
  flag remains compatible.
- Keep `response.output_item.done` items even when the terminal response's
  `output` array is empty, as observed on the live ChatGPT route.
- Return executable calls only after `response.completed`. Failed, incomplete
  and interrupted streams remain distinct. A ChatGPT usage-limit error stops
  inference and points to usage settings, with no billing fallback or OAuth loop.
- Keep account-specific prices unknown; never report an invented zero charge.

The live account catalog, inspected on October 25, 2026, includes
`context_window`, `supported_reasoning_levels[{effort, description}]` and
`default_reasoning_level`. The adapter caches these facts within the selected
account's API handle. It uses the effective `context_window`, rather than the
larger `max_context_window`, for the meter and automatic compaction. The effort
picker follows the server's supported levels, including `ultra` when offered.
Missing capacity remains unknown; missing effort metadata keeps the provider
default and permits explicit requests through `/effort NAME`.

## Settings

`Settings > Providers` separates **Credentials > OpenAI** from
**API keys > OpenRouter**. ChatGPT sign-in uses **Continue with ChatGPT**.
When multiple providers are configured, `/model` selects the provider before
the model and effort. The active plan is identified in the interface and usage
management links to `https://chatgpt.com/settings/usage`.

## Request and cache behavior

Keep the local source transcript independent from prompt caching. This HTTP
route omits `previous_response_id` and replays the required input items; the
local archive remains the recoverable copy.

Use the selected model's documented [prompt caching](https://developers.openai.com/api/docs/guides/prompt-caching)
behavior. Do not copy Anthropic `cache_control` fields into an OpenAI request.
Keep tool definitions and reusable prefixes stable, preserve opaque provider
items in their owning adapter, and record reported cache reads and writes.
Assess connection reuse separately from cache hits and from context compaction.

Each request carries the conversation ID as `prompt_cache_key` and as the
`session-id` header to support cache affinity. A stable key does not guarantee
a cache hit; OpenAI routes requests automatically, and newer models may not
need the key. Reported `cached_tokens` appear with the completion. The stream
reports progress when a reasoning or tool-call item opens, before its first
delta, so the TUI leaves "Waiting for model" as soon as work starts.
Separate assistant message items have a blank line between them in both the
streamed display and the saved portable text; native items retain their phases.

The access token is kept in memory until it nears expiry or the account record
file changes. Cache hits still check the record file's metadata, but skip the
locked record read, JSON parsing and native permission/hash helpers.

Temporary HTTP and stream failures retain the server's `Retry-After` delay
(seconds or HTTP date). The recovery loop waits at least that long and remains
cancellable. `slow_down` and `server_is_overloaded` stream errors are temporary;
a plan usage-limit error still stops inference.
An unrepresentable retry deadline omits only the TUI countdown; the server's
minimum delay is retained and user cancellation still interrupts the wait.
The top-level `error` event's code, message and parameter are retained alongside
nested response errors. Successful SSE bodies go directly to the stream parser;
the process capture and HTTP error body each retain at most 64 KiB of diagnostics.
