# Anthropic subscription backend

The Anthropic adapter uses a Claude account alongside OpenRouter and ChatGPT.
It has no Anthropic API-key path. Authentication and Messages translation belong
to the adapter; Jecode's agent loop, tools, context management, collaboration and
local conversations remain shared.

## Compatibility status

This native compatibility route was verified on October 31, 2026.
It is not an approved third-party Sign in with Claude integration. Anthropic's
[Agent SDK documentation](https://code.claude.com/docs/en/agent-sdk/overview)
states that third-party products need prior approval to offer claude.ai login
or subscription rate limits. A successful request establishes technical
interoperability, not provider approval or future availability.

Jecode calls the service through its own native HTTP boundary. It neither
installs nor invokes Claude Code, an Agent SDK, another agent CLI or a proxy.
The OAuth authorization page identifies the public client as Claude Code.
Generation requests use this compatibility profile:

- Public client ID: `9d1c250a-e61b-44d9-88ed-5944d1962f5e`.
- `User-Agent: claude-cli/2.1.314`, pinned to the reference tested locally.
- `x-app: cli` and `anthropic-version: 2023-06-01`.
- `anthropic-beta: claude-code-20250219,oauth-2025-04-20`.
- The system prefix `You are Claude Code, Anthropic's official CLI for Claude.`
  occupies its own text block before Jecode's actual instructions. Existing system and developer
  instructions and the original tool schemas remain present.

The wire identity is compatibility behavior, not a description of the running
application. Account and catalog requests identify Jecode. Anthropic can reject
or change any part of this route independently of Jecode releases.

## Account lifecycle

`auth/anthropic/` starts an IPv4 loopback listener before opening the system
browser at `https://claude.ai/oauth/authorize`. Each attempt has fresh state and
PKCE S256 material. The callback uses `http://localhost:<port>/callback`, checks
the host, path, state and unique parameters, and expires after five minutes.
Invalid callbacks do not terminate the pending attempt. Users can cancel it.

Code exchange and refresh use
`https://platform.claude.com/v1/oauth/token`, with the scopes `user:profile`
and `user:inference`. The profile endpoint
`https://api.anthropic.com/api/oauth/profile` supplies the stable account UUID
and display label. Returning sign-in must match the selected saved identity.
Jecode does not inspect another client's credentials.

Account records are plaintext JSON under `~/.jecode/accounts/anthropic/`,
separate from `config.json`. Unix uses `0700` directories and `0600` files;
Windows applies an ACL restricted to the current user. An OS file lock
serializes account updates and refresh. A refresh rereads the saved record and
atomically replaces the access token, rotating refresh token, expiry and scopes.
Shared redaction sources include both old and new token values.

Sign-out removes local tokens and clears the active account under the lock,
even if cancellation was already requested. It retains the account label for
returning sign-in. It does not claim to revoke the remote authorization.
Malformed unselected records are skipped and counted at startup; access errors
and a corrupt selected account remain explicit errors.

## Messages route

- List `GET https://api.anthropic.com/v1/models` with the selected account.
  Use reported input/output limits and effort capabilities; unknown limits and
  subscription prices remain unknown.
- Send `POST https://api.anthropic.com/v1/messages?beta=true` with streaming
  enabled, the selected model and the canonical conversation.
- Translate local tools to `input_schema`, tool calls to `tool_use` and adjacent
  tool receipts to `tool_result` blocks. Preserve signed thinking and ordered
  content blocks for replay only to the same backend, account and model.
- Assemble fragmented tool JSON and require `message_stop` before returning
  executable calls. Truncated streams, output limits, context limits,
  cancellation and provider errors retain their distinct recovery behavior.
- Materialize images and PDFs at the adapter boundary. Keep unsupported file
  types as attachment descriptions.
- Normalize input usage as uncached input plus cache reads plus cache writes.
  Preserve reported cache counters without inventing a subscription charge.
- Opt in to prompt caching at the stable system boundary and recent durable
  user-message ends, with at most four cache breakpoints. The final request-only
  session state stays outside them. Retaining earlier boundaries covers large
  batches of tool results beyond the provider's 20-content-block lookup window.
  Native signed thinking blocks stay unchanged.

On documented models with mid-conversation system-message support, the
request-local state is a final `role: "system"` message within `messages`.
Keeping it separate avoids normalizing adjacent user messages differently as
the durable conversation grows. Unknown or older model IDs retain the user
fallback. This does not change the top-level compatibility system prefix.

Requests use the account's usage settings. Jecode does not enable extra usage,
modify spending limits or silently switch to API billing. Provider rate limits
are reported through the existing cancellable retry path, including a supplied
`Retry-After` delay.

## Verification

Offline fixtures use original Rust servers, synthetic credentials and isolated
homes. They exercise the build-embedded native HTTP adapter, account persistence
and sign-out, callback and token validation, model metadata, signed thinking,
tool replay, cache usage and rate-limit handling.

Before integration, an isolated native experiment completed one Opus 5.5
streamed text request with extra usage verified disabled before and after the
request. Four earlier requests using Jecode's generation identity returned
HTTP 429 while OAuth, usage and catalog requests succeeded. The successful run
also happened after a usage-window reset, so it does not isolate which
compatibility field changed the outcome. Those prototype runs alone did not
establish the integrated tool loop or refresh path.

Integrated verification on Windows used a separate `JECODE_HOME` and a
synthetic project. Browser login and model listing succeeded. After a first
full request returned HTTP 429, the compatibility prefix was put in its own
system text block and a fresh account session was used. The minimal stream,
two-request signed tool replay and ordinary CLI `read` turn all succeeded.
Forcing the isolated record's expiry exercised a real refresh: the refresh
token rotated, expiry advanced and account identity stayed unchanged. A
separate CLI process resumed the saved read conversation and returned the
previous tool result without another tool call. Extra usage was verified
disabled for the bounded live fixtures.

These checks establish that path with Opus 5.5 on Windows. Other models,
attachments and live account access on Linux/macOS were not exercised.
Offline fixtures establish their own narrower contracts. Use the opt-in
checks in [testing](TESTING.md) to repeat the bounded stream/tool checks
without automatic retries.
