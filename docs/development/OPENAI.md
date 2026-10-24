# OpenAI backend direction

The OpenAI backend is planned, not implemented. Jecode currently ships the
OpenRouter adapter. Preserve the independent `Backend` and `Service` boundaries:
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
  expiry together. Preserve account/workspace separation and the retained ID
  token for later `id_token_hint` use. Never log credentials.

The Rust standard library does not supply HTTPS or JWT signature verification.
The implementation must define and test its system API/tool boundary, including
availability and cancellation, rather than omit those checks.

## Request and cache behavior

Keep provider conversation state separate from the local source transcript and
from prompt caching. In particular, `previous_response_id` does not make earlier
input free; see [conversation state](https://developers.openai.com/api/docs/guides/conversation-state).
An adapter may retain provider state without making it the only recoverable copy.

Use the selected model's documented [prompt caching](https://developers.openai.com/api/docs/guides/prompt-caching)
behavior. Do not copy Anthropic `cache_control` fields into an OpenAI request.
Keep tool definitions and reusable prefixes stable, preserve opaque provider
items in their owning adapter, and record reported cache reads and writes.
Assess connection reuse separately from cache hits and from context compaction.
