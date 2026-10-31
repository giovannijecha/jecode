# Anthropic backend

The Anthropic backend sends requests with a Claude subscription account signed
in through claude.ai. It has no API-key mode. Authentication and
Messages translation belong to the adapter in `auth/anthropic/` and
`backend/anthropic/`; the agent loop, tools, context management, collaboration
and saved conversations are shared with the other backends.

User setup is described in [Providers](../usage/PROVIDERS.md#claude-account).

## Compatibility status

This is a compatibility route, not an approved integration. Anthropic's
[Agent SDK overview](https://code.claude.com/docs/en/agent-sdk/overview) states
that, unless previously approved, Anthropic does not allow third-party
developers to offer claude.ai login or rate limits for their products. Jecode
has no such approval. A working request shows technical interoperability only;
Anthropic can restrict or reject the route at any time, independently of Jecode
releases.

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
3. The code is exchanged at `https://platform.claude.com/v1/oauth/token` for the
   scopes `user:profile` and `user:inference`.
4. `https://api.anthropic.com/api/oauth/profile` supplies the stable account
   UUID and display label. Signing in again must match the selected saved
   identity.

Account records are plaintext JSON under `~/.jecode/accounts/anthropic/`,
separate from `config.json`. Newly created Unix directories use mode `0700` and
files `0600`; on Windows the files get an ACL restricted to the current user. An
OS file lock serializes updates and refreshes. A refresh rereads the record and
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
  account and model.
- **Completion.** Fragmented tool JSON is reassembled, and calls are returned
  only after `message_stop`. Truncated streams, output limits, context limits,
  cancellation and provider errors keep distinct recovery paths.
- **Attachments.** Images and PDFs are materialized at the adapter boundary;
  other file types are described in the attachment manifest.
- **Usage.** Input is normalized as uncached input plus cache reads plus cache
  writes. Reported cache counters are kept, and no subscription charge is
  invented.
- **Caching.** Requests opt in to prompt caching at the stable system boundary
  and recent durable user-message ends, with at most four breakpoints. The final
  request-only session state stays outside them. On models documented to accept
  mid-conversation system messages, that state is a final `role: "system"`
  message; other models receive it as a user message.

Effort and adaptive thinking are separate catalog capabilities. Adaptive
thinking is sent only when the model reports support for it. Compaction prefers
medium effort and conversation titles low effort, each falling back to the
default when the catalog does not list that level. See Anthropic's
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

The live route has been exercised on Windows only, with an isolated
`JECODE_HOME`, Opus 5.5 and extra usage verified as disabled: browser sign-in,
model listing, a streamed reply, a two-request signed tool round trip, an
ordinary CLI `read` turn, a forced token refresh with rotation, and resuming
that conversation from a separate process. Other models, attachments, and live
use on Linux or macOS have not been exercised.

The opt-in live checks are listed in
[Development and testing](TESTING.md#live-provider-checks).
