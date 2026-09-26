# TLS profile

Jecode implements its own authenticated TLS 1.3 client. The profile is X25519,
`TLS_AES_128_GCM_SHA256` and HTTP/1.1. Native APIs provide randomness and trust
roots; handshake, record protection, signature checks and certificate-path
decisions are owned code.

This is experimental infrastructure without an independent security audit.
Protocol vectors and interoperability checks do not establish general WebPKI
compatibility, certification or a side-channel audit.

## Supported path

The handshake validates negotiation, server Finished, certificate identity/path
and CertificateVerify before releasing application keys. AES-GCM authenticates
records before delivering plaintext. Sequence numbers and key epochs cannot be
reset or cloned. NewSessionTicket messages are bounded and discarded; KeyUpdate
updates the relevant keys in protocol order. Unsupported messages fail closed.

The client offers ECDSA-P256-SHA256 and RSA-PSS-SHA256 handshake verification.
Certificate signatures support ECDSA P-256/P-384 with SHA-256/SHA-384 and bounded
RSA 2048-4096 verification with PKCS#1 v1.5 or the supported PSS profile.
Certificates require canonical DER, matching DNS SAN, validity, key purpose,
issuer signatures and CA/path constraints. Wildcards consume one leftmost label;
there is no Common Name fallback.

Windows trust uses CurrentUser/LocalMachine ROOT and Disallowed data through
Crypt32. Linux reads the supported system PEM certificate bundle. Jecode ships
no bundled roots. Enterprise roots installed locally can authorize interception;
certificate verification failures are not silently accepted.

## Limits

There is no HelloRetryRequest, PSK/session resumption, client-certificate support,
HTTP/2, AIA fetching, online OCSP/CRL checking or certificate-transparency enforcement.
Unsupported name/policy constraints and must-staple fail closed. This profile does
not establish fresh certificate revocation status. Certificate RSA-PSS algorithm
parameters outside the implemented profile are unsupported.

Parsing, records, certificate bodies and path search have explicit bounds.
Connections are single-use. A model terminal event can establish completion before
transport closure; the connection is then disposed rather than pooled.

## Interrupted request diagnostics

After closing Jecode, a source checkout can print the latest turn's last 32
network attempts without exporting a session log:

```text
cargo run --locked --offline --bin jecode-check -- account-attempts SESSION_ID DIRECTORY
```

`DIRECTORY` must be the session's selected working directory. The report contains
only fixed labels, numeric counters and a validated HTTP status. Request sequence
is within the turn; connection attempt is within that request. Stage time is
wall time in the failed connect, write or response-read stage, including local
decoding and progress delivery. `request_ms` is elapsed time for that connection
attempt; `since_progress_ms` counts receive-eligible time since the completed
write or last accepted SSE event, excluding synchronous local response parsing
and presentation delivery. `termination` distinguishes setup, write,
first-response and stream-idle timeouts, explicit total-budget expiry and
cancellation. Older records have no termination label or progress age. Accepted
TLS write bytes are bytes accepted by
the local socket, including TLS framing. Received TLS wire bytes are bytes
returned by local TCP reads after the request write, including incomplete records.
Decrypted HTTP bytes were handed to the HTTP decoder; SSE events reached the
model event decoder. None of these byte counts proves remote processing.
The report omits credentials, prompts, response text, tool arguments and raw
session contents.

For a provider-reported stream failure, `provider_event` records either
`response.failed` or `error` and `provider_code` records an allowlisted error
code. The code is read from `response.error.code` for `response.failed` and from
the top-level `code` for `error`. Missing or unrecognized codes are `unknown`;
wrongly typed or oversized codes are `malformed`. Older attempts and transport
failures have `unknown` metadata. Raw provider messages and unrecognized code
strings are discarded, and these labels do not trigger retries. The allowlist
includes `server_error`, `rate_limit_exceeded`, `slow_down`,
`server_is_overloaded`, `context_length_exceeded`, `insufficient_quota`,
`credit_balance_exhausted`, `organization_spend_limit_exceeded`,
`project_spend_limit_exceeded`, `organization_usage_limit_exceeded`,
`usage_not_included` and `invalid_prompt`.
The event shapes and `server_error` example follow the
[Responses streaming reference](https://platform.openai.com/docs/api-reference/responses-streaming).
The other allowlisted codes are backed by the
[API error guide](https://developers.openai.com/api/docs/guides/error-codes) or
the inspected upstream Codex parser at
[`e72da2b53805894878023d01949a25a082e0a5cb`](https://github.com/openai/codex/blob/e72da2b53805894878023d01949a25a082e0a5cb/codex-rs/codex-api/src/sse/responses_error.rs).

Cancellation and deadlines are checked during connect, reads and writes. The
model request uses finite setup and write windows, followed by a configurable
first-event and established-stream inactivity window. A complete accepted SSE
data event renews only the latter window; incomplete records and keepalive
comments do not. Response parsing and the bounded presentation queue can stop
the reader temporarily; that local time is excluded from the inactivity window
without inventing a new SSE event. Memory remains bounded by the existing
64-event queue and protocol limits. Cancellation and shutdown interrupt queue
delivery; an explicit caller total deadline still uses wall time. If presentation
never drains, provider progress cannot be observed until the reader resumes. The
standard-library DNS resolver is synchronous and cannot be interrupted inside its
OS call. Cleanup remains joined. Bounded CPU verification is not interruptible at
every instruction. Memory clearing cannot guarantee removal of compiler, allocator,
OS or crash-dump copies of secrets.

The implementation has offline known-answer and negative tests, including TLS
1.3 handshake vectors, record tampering, invalid signatures and certificate/path
rejection. The public test suite uses synthetic fixtures and never real account
credentials.
