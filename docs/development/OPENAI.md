# OpenAI backend

The OpenAI backend sends requests with a ChatGPT account through Sign in with
ChatGPT. It has no API-key mode. Authentication, wire format, provider state and
caching belong to the adapter in `auth/openai/` and `backend/openai/`; tools,
canonical history and terminal rendering are shared with the other backends.

User setup is described in [Providers](../usage/PROVIDERS.md#chatgpt-account).

## Sign in with ChatGPT

Jecode follows OpenAI's documented
[sign-in flow for open-source apps](https://developers.openai.com/siwc/token-sharing-open-source/sign-in)
as a native public client with its own identity, without a client secret or
partner API key. It never reads Codex credentials, reuses a Codex client ID or
depends on the Codex CLI.

1. Jecode keeps a stable `ext_agent_host_id` for the installation. The first
   registration sends `client_id=dynamic_agent_client` and
   `agent_name_hint=Jecode`, and receives an issued client ID that is used for
   the code exchange and kept with the account.
2. A loopback listener starts on `127.0.0.1` with a random port before the
   system browser opens. The redirect URI is
   `http://127.0.0.1:<port>/auth/callback`. Each attempt has fresh state, an
   OIDC nonce and a PKCE S256 verifier, and expires after five minutes.
   Unrelated or invalid callbacks receive HTTP 400 without ending the attempt;
   a matching provider error ends it.
3. Jecode verifies the ID token's signature against OpenAI's JWKS, along with
   the issuer, the issued-client audience, expiry and nonce. The verified
   subject becomes the account identity, and signing in again must match the
   selected account.
4. Jecode requests the documented identity and plan-use scopes for the resource
   `https://api.openai.com/v1`, and requires the granted
   `chatgpt.tokens.use.direct` scope before inference.

Account records are plaintext JSON under `~/.jecode/accounts/openai/`, separate
from `config.json`. Newly created Unix directories use mode `0700` and files
`0600`; on Windows the files get an ACL restricted to the current user. The host
identity and each verified registration survive sign-out. Refresh rereads the
record under an OS file lock and replaces the rotating token set atomically. An
omitted or null `refresh_token` in a refresh response keeps the previous one, as
[RFC 6749 section 6](https://www.rfc-editor.org/rfc/rfc6749#section-6) allows.
The access token stays in memory until it nears expiry or the record file
changes.

Sign-out removes local credentials under the account lock, even after
cancellation, then releases the lock and attempts remote revocation. At startup,
malformed records are skipped and counted; filesystem errors and a corrupt
selected account remain explicit errors. The account picker keeps
registrations apart even when they share an email address.

During `jecode setup`, a scoped console reader lets Enter cancel a pending
sign-in. It is stopped and reaped before setup reads the next prompt.

## Cryptography and transport

Random bytes, SHA-256 and RSA signature checks use Windows .NET cryptography,
macOS Security and CommonCrypto, or the system Linux libcrypto. HTTPS uses
Windows WinHTTP, macOS CFNetwork, or Linux sockets with the system libssl and
its certificate and hostname verification. The adapters are compiled at build
time and embedded in the executable. Request bodies and bearer tokens reach them
on stdin, never in arguments or files. No external crate or copied
cryptographic code is used.

## Responses route

OpenAI's
[preview limitations](https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations)
define this route; it is narrower than the API-key Responses API.

- **Catalog.** `GET https://api.openai.com/v1/models` with the account token.
  Jecode keeps models with `visibility: "list"` in server order, and uses each
  model's `context_window` (not the larger `max_context_window`),
  `supported_reasoning_levels` and `default_reasoning_level`. Missing capacity
  stays unknown; missing effort metadata keeps the provider default, and
  `/effort NAME` can still request a level explicitly.
- **Generation.** `POST https://api.openai.com/v1/responses` with
  `stream: true`, `store: false` and the full required history in `input`. No
  continuation IDs, server conversations, output-token caps or unsupported
  sampling parameters are sent.
- **Tools.** System instructions become `instructions`; local calls and results
  become function-call items, with function definitions grouped in a namespace.
- **Native state.** Ordered output items, encrypted reasoning and assistant
  phases are preserved and replayed only to the same account and model.
  `response.output_item.done` items are kept even when the final response's
  `output` array is empty.
- **Completion.** Calls are returned only after `response.completed`. Failed,
  incomplete and interrupted streams stay distinct.
- **Usage limits.** The error code
  `subscription_sharing_usage_limit_exceeded` stops inference and points to
  `https://chatgpt.com/settings/usage`. Other HTTP 429 responses, `slow_down`
  and `server_is_overloaded` are retried, honoring `Retry-After` in seconds or
  as an HTTP date.
- **Prices.** Account prices stay unknown; Jecode never reports an invented zero
  charge.

## Caching

The local transcript is the recoverable copy and is independent of provider
caching. Each request sends the conversation ID as `prompt_cache_key` and as the
`session-id` header for cache affinity; a stable key does not guarantee a cache
hit. The subscription route rejected explicit `prompt_cache_options`, so Jecode
relies on implicit caching and places request-only session state in a final
developer message, after the stable prefix. Reported `cached_tokens` appear with
the completion.

The stream reports progress as soon as a reasoning or tool-call item opens, so
the TUI leaves "Waiting for model" when work starts. Separate assistant message
items are joined with a blank line in both the display and the saved text.

## Verification

Offline fixtures use isolated account records and loopback HTTP. They never read
Codex credentials or start a real sign-in, and a passing run does not establish
account eligibility or real plan inference. The opt-in live checks use the
selected saved ChatGPT account and consume plan usage; see
[Development and testing](TESTING.md#live-provider-checks).
