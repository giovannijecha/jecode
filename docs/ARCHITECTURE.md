# Architecture

Jecode is one Rust package with no external normal, build or test dependencies.
The model-facing controller is deliberately centralized; its supporting modules
have separate responsibilities.

| Boundary | Responsibility |
| --- | --- |
| `src/main.rs` | CLI parsing and selected workspace |
| `src/terminal/` | Input, inline layout and streamed rendering |
| `src/session/` | Ordered conversation, tool loop, metrics and context |
| `src/session/persistence/` | Versioned canonical history and resume |
| `src/state/` | User-scoped files, settings, permissions and file locks |
| `src/providers/openai_account/` | Authentication, credential refresh and Responses protocol |
| `src/tools/` | Small model-facing schema and validated dispatch |
| `src/workspace/` | Bounded reads, search and recoverable file changes |
| `src/command/` | Shell execution, output and joined process cleanup |
| `src/http/`, `src/stream/`, `src/json/`, `src/tls/` | Owned protocol implementations |
| `tests/`, `checks/`, `.github/` | Reproducible verification and CI |

The terminal sends task input to the controller and presents events. It does not
execute tool effects. The worker owns prepared operations and executes valid model
calls in order. It checkpoints an uncertain receipt before each effect and the
exact outcome afterward. A failed required checkpoint stops later effects.
Cancellation reaches
the provider, tools and process owner, and closing the UI joins its worker.

The workspace owns a starting directory and a separate file-access profile.
Path resolution and native opens enforce that profile; changing a command's
directory does not change the file-access profile. Commands run with the user's
permissions and are not sandboxed.
Sessions store the profile, while model instructions derive from the active
workspace so request-size measurements include the same environment description.

Canonical history holds user messages, accepted guidance, provider output,
tool receipts and turn outcomes. Requests project validated assistant items and
paired tool results. Partial output is not invented as a completed model response.
Resume loads facts and waits for new user input. Context compaction stores a
separate summary and cutoff without removing canonical turns.

Persistence uses bounded versioned records and OS file leases. New sessions append
canonical events to a log and atomically replace a small committed head. v1 JSON
snapshots remain readable and require explicit verified import for v2 continuation.
See [session recovery](SESSIONS.md) for commit and native durability boundaries.
Credentials use a separate file and never enter session encoding.
The refresh lease covers loading and replacement, not model generation. Concurrent
instances reload account state before each model request.

The terminal adapts canonical session events to typed transcript blocks and
owns a bounded FIFO of prompts entered during generation. It submits each
pending prompt as a new turn after the previous turn completes. The renderer
uses synchronized paints and relative tail updates in the normal buffer. A
resize previews one screen and then replays the transcript at the settled
width; Ctrl+O uses the same replay path for global tool-detail expansion.
The renderer supports no-color and reduced-motion modes and has no authority
to read files or run commands.

The compiler, Cargo, linker, OS and CI runner are infrastructure. Native APIs are
used for certificate roots, randomness, terminal control, filesystem operations,
private permissions and process lifecycle. The TLS handshake, encryption,
certificate validation, HTTP, streaming and application behavior are owned code;
see [TLS](TLS.md) and [platform support](COMPATIBILITY.md).
