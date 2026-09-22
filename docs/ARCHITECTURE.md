# Architecture

Jecode is one Rust package with no external normal, build or test dependencies.
The model-facing controller is deliberately centralized; its supporting modules
have separate responsibilities.

| Boundary | Responsibility |
| --- | --- |
| `src/main.rs` | CLI parsing and selected workspace |
| `src/terminal/` | Input, inline layout, streamed rendering and approval presentation |
| `src/session/` | Ordered conversation, tool loop, queued guidance, metrics and context |
| `src/session/persistence/` | Versioned canonical history and resume |
| `src/state/` | User-scoped files, settings, permissions and file locks |
| `src/providers/openai_account/` | Authentication, credential refresh and Responses protocol |
| `src/tools/` | Small model-facing schema and validated dispatch |
| `src/workspace/` | Bounded reads, search and recoverable file changes |
| `src/command/` | Approved shell execution, output and process cleanup |
| `src/http/`, `src/stream/`, `src/json/`, `src/tls/` | Owned protocol implementations |
| `tests/`, `checks/`, `.github/` | Reproducible verification and CI |

The terminal sends intent to the controller and presents events. It does not
execute tool effects. Exact proposals stay with the worker; approval messages
contain an operation ID and a decision. Effects are ordered. Cancellation reaches
the provider, tools and process owner, and closing the UI joins its worker.

Canonical history holds user messages, accepted guidance, provider output,
tool receipts and turn outcomes. Requests project validated assistant items and
paired tool results. Partial output is not invented as a completed model response.
Resume loads facts and waits for new user input. Context compaction stores a
separate summary and cutoff without removing canonical turns.

Persistence is ordinary JSON, bounded and versioned, with atomic replacement and
OS file leases. Credentials use a separate file and never enter session encoding.
The refresh lease covers loading and replacement, not model generation. Concurrent
instances reload account state before each model request.

The renderer keeps stable output in native scrollback and redraws a bounded
transient area for activity, approvals and the composer. It supports no-color and
reduced-motion modes. Rendering has no authority to read files or run commands.

The compiler, Cargo, linker, OS and CI runner are infrastructure. Native APIs are
used for certificate roots, randomness, terminal control, filesystem operations,
private permissions and process lifecycle. The TLS handshake, encryption,
certificate validation, HTTP, streaming and application behavior are owned code;
see [TLS](TLS.md) and [platform support](COMPATIBILITY.md).
