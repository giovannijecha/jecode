# Anthropic backend

The Anthropic adapter sends Messages requests with a direct API key
(`anthropic-api`) or a Claude subscription account signed in through claude.ai
(`anthropic`). Authentication and
Messages translation belong to the adapter in `auth/anthropic/` and
`backend/anthropic/`; the agent loop, tools, context management, collaboration
and saved conversations are shared with the other backends.

User setup is described in [Providers](../usage/PROVIDERS.md#claude-account).

## Direct API key

`anthropic-api` uses the [Claude API](https://platform.claude.com/docs/en/api/overview)
with a Bearer API key and `anthropic-version: 2023-06-01`. Its normal
`/v1/messages` route sends Jecode's identity, without OAuth beta headers,
`x-app: cli` or a Claude Code system prefix. It shares model metadata,
streaming, tool translation and cache accounting with the account adapter.

API keys have separate hashed identities for provider-owned history. For
storage and access permissions, see [Credentials](../../SECURITY.md#credentials).
The adapter supports workspace-scoped
keys; it does not currently configure the workspace header required by unscoped
keys. An API rejection never causes a retry with subscription credentials.

## Compatibility status

Read the [Claude account caveat](../usage/PROVIDERS.md#claude-account) before
using the subscription route. The technical compatibility profile is below.

Jecode calls the service through its own native HTTP adapter. It does not
install or run Claude Code, an Agent SDK, another agent CLI or a proxy, and it
never reads another client's credentials. The OAuth authorization page names
the public client as Claude Code, and generation requests present this profile:

| Field | Value |
| --- | --- |
| OAuth client ID | `9d1c250a-e61b-44d9-88ed-5944d1962f5e` |
| `User-Agent` | `claude-cli/2.1.314` |
| `x-app` | `cli` |
| `anthropic-version` | `2023-06-01` |
| `anthropic-beta` | `claude-code-20250219,oauth-2025-04-20` |
| System prefix | `You are Claude Code, Anthropic's official CLI for Claude.` in its own text block, before Jecode's instructions |

The profile applies only to `POST /v1/messages`. Account, profile and catalog
requests identify as `jecode/0.1` and send only the `oauth-2025-04-20` beta.

## Account lifecycle

1. Jecode opens a loopback listener on `127.0.0.1` with a random port, then
   opens the system browser at `https://claude.ai/oauth/authorize`.
2. Each attempt uses fresh state and a PKCE S256 verifier. The redirect URI is
   `http://localhost:<port>/callback`; the listener checks the `Host` header,
   path, state and that each parameter appears once. Invalid callbacks do not
   end the attempt. The attempt expires after five minutes and can be
   cancelled.
   The browser of a valid callback waits until the tokens are saved; declined and
   invalid callbacks are answered at once. Either way it gets the same black
   `jecode authenticated` or `jecode not authenticated` page as the other
   loopback sign-ins (`auth/landing.rs`).
3. The code is exchanged at `https://platform.claude.com/v1/oauth/token` for the
   scopes `user:profile` and `user:inference`.
4. `https://api.anthropic.com/api/oauth/profile` supplies the stable account
   UUID and display label. Signing in again must match the selected saved
   identity.

Credential storage and access permissions are described in
[Security](../../SECURITY.md#credentials). An OS file lock serializes account
updates and refreshes. A refresh rereads the record and
atomically replaces the access token, the rotating refresh token, the expiry
and scopes. Redaction keeps both old and new token values masked.

Sign-out removes the local tokens and clears the active account under the lock,
keeping the account label for a later sign-in. It does not revoke the remote
authorization. At startup, malformed unselected records are skipped and
counted; access errors and a corrupt selected account remain explicit errors.

## Messages route

- **Catalog.** `GET https://api.anthropic.com/v1/models` with the selected
  account. Jecode uses the reported input and output limits and effort
  capabilities. Unknown limits and subscription prices stay unknown.
- **Generation.** `POST https://api.anthropic.com/v1/messages?beta=true` with
  streaming enabled.
- **Tools.** Local tools become `input_schema` definitions, calls become
  `tool_use` blocks and adjacent results become `tool_result` blocks. Signed
  thinking and ordered content blocks are replayed only to the same backend,
  account and model, exactly as received: tool input normalized for execution
  stays in the canonical call. Signed thinking binds everything sent before
  it, so image and PDF views from tools stay in every later request.
- **Completion.** Fragmented tool JSON is reassembled, and calls are returned
  only after `message_stop`. Truncated streams, output limits, context limits,
  cancellation and provider errors keep distinct recovery paths.
- **Attachments.** Images and PDFs are materialized at the adapter boundary;
  other file types are described in the attachment manifest.
- **Usage.** Input is normalized as uncached input plus cache reads plus cache
  writes. Reported cache counters are kept, and no subscription charge is
  invented.
- **Caching.** Requests opt in to prompt caching at the stable system boundary,
  two history breakpoints on recent durable user-message ends and the automatic
  top-level breakpoint, which covers the end of the request. Runtime session state is
  anchored where it was first sent and added again only when it changes, so it
  stays in the cacheable history; only state sent for the first time at the end
  is covered by the automatic breakpoint alone. On the Claude plan, for models
  documented to accept mid-conversation system messages, state that follows a
  user turn is sent as a `role: "system"` message; otherwise, including a
  continuation after an answer, it is a user message. The shared built-in
  block always lasts one hour. The other breakpoints follow the saved **Prompt
  cache** lifetime: the provider's five minutes by default, or one hour when chosen.
  Requests without that setting, such as standalone maintenance, use five minutes.

Effort and adaptive thinking are separate catalog capabilities. Adaptive
thinking is sent only when the model reports support for it. Compaction first
asks over the conversation's own request, with its tools and selected effort;
when it falls back to portion summaries, it prefers medium effort without tools.
Conversation titles use low effort. Each falls back to the default when the
catalog does not list that level. See Anthropic's
[thinking](https://platform.claude.com/docs/en/build-with-claude/thinking) and
[effort](https://platform.claude.com/docs/en/build-with-claude/effort)
documentation.

Requests use the account's own usage settings. Jecode does not enable extra
usage, change spending limits or switch to API billing. Rate limits go through
the normal cancellable retry path and honor `Retry-After`.

## Verification

Offline fixtures use original Rust servers, synthetic credentials and isolated
homes. They cover the native HTTP adapter, account persistence and sign-out,
callback and token validation, model metadata, signed thinking, tool replay,
cache usage and rate-limit handling. They never read Claude Code credentials or
start a real sign-in.

The account route has been exercised live on Windows only. With an isolated
`JECODE_HOME`, Opus 5.5 and extra usage verified as disabled, that covered browser
sign-in, model listing, a streamed reply, a two-request signed tool round trip, an
ordinary CLI `read` turn, a forced token refresh with rotation, and resuming that
conversation from a separate process. Sonnet 5.5 ran the transport probe in
[Performance](../reports/PERFORMANCE_RESULTS.md#provider-transports) and the
[benchmark](../reports/AGENT_BENCHMARK_2026-11.md). The API-key route has also run live conversations
under WSL; see [Platform support](../usage/PLATFORMS.md). Attachments, other models, the
account route on Linux, and any live use on macOS have not been exercised.

The opt-in live checks are listed in
[Development and testing](TESTING.md#live-provider-checks).
