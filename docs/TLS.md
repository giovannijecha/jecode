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
attempt; `since_progress_ms` is time since the completed write or last accepted
SSE event. `termination` distinguishes setup, write, first-response and stream-idle
timeouts, explicit total-budget expiry and cancellation. Older records have no
termination label or progress age. Accepted TLS write bytes are bytes accepted by
the local socket, including TLS framing. Received TLS wire bytes are bytes
returned by local TCP reads after the request write, including incomplete records.
Decrypted HTTP bytes were handed to the HTTP decoder; SSE events reached the
model event decoder. None of these byte counts proves remote processing.
The report omits credentials, prompts, response text, tool arguments and raw
session contents.

Cancellation and deadlines are checked during connect, reads and writes. The
model request uses finite setup and write windows, followed by a configurable
first-event and established-stream inactivity window. A complete accepted SSE
data event renews only the latter window; incomplete records and keepalive
comments do not. An explicit caller total deadline still takes precedence. The
standard-library DNS resolver is synchronous and cannot be interrupted inside its
OS call. Cleanup remains joined. Bounded CPU verification is not interruptible at
every instruction. Memory clearing cannot guarantee removal of compiler, allocator,
OS or crash-dump copies of secrets.

The implementation has offline known-answer and negative tests, including TLS
1.3 handshake vectors, record tampering, invalid signatures and certificate/path
rejection. The public test suite uses synthetic fixtures and never real account
credentials.
