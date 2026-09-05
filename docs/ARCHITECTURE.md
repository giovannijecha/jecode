# Architecture

This guide explains the current implementation and its module boundaries.
The [compatibility contract](COMPATIBILITY.md) defines which public behavior
must remain supported; layout and implementation details here describe the
current code and evolve with it.

Read by concern: [turn execution](#one-turn), [foreground activity](#foreground-activity),
[providers](#provider-boundary), [workspace access](#workspace-boundary),
[local data](#settings-credentials-and-local-data),
[sessions and recovery](#conversation-state-and-resume),
[terminal composition](#tui-composition), or [validation](#validation-boundaries).

## Mental model

Jecode has one agent loop. The TUI supplies a user message;
the controller sends it to one provider, executes requested tools, and repeats
until the model returns no tool calls.

```text
bin/jecode.js → dist/main.js → start.js
                               └─ tui/app.js       interactive control plane
                                    ├─ app-input.js
                                    └─ app-workflows.js
                                         ├─ command-workflow.js ─ commands.js
                                         └─ turn-workflow.js
                                              └─ controller.js   the only agent loop
                                                   ├─ providers/ wire-format boundaries
                                                   └─ tools/     workspace capabilities

npm run start → src/main.ts    direct development path
npm run pack:release → clean build → dist/ inside the release tarball
```

`start.ts` handles help and version before configuration or terminal takeover.
Every conversation launch then requires a TTY on both stdin and stdout before
loading settings, selecting a provider, or opening session storage. A failed
terminal check returns a stderr diagnostic and non-zero exit; there is no
fallback execution surface.

There are no subagents, workers, delegated tasks, or concurrent controller
loops. One model response may overlap consecutive read-only tool calls inside a bounded
four-call wave; writes, commands, approvals, and unknown tools remain ordered
barriers. Complex work advances through more iterations of the same loop.

## One turn

`src/prompt.ts` contributes runtime context plus tool and response rules. It
does not assign an assistant or product identity to the model or automatically
ingest repository files. Project identity enters context only through
workspace content the model reads or content the user supplies.

`runTurn` owns the protocol:

1. Materialize the current model-facing context projection, then send it with
   tool declarations and request settings to the selected provider.
2. Stream display events while the provider assembles the authoritative
   assistant message.
3. Append that message to history and record normalized usage.
4. When it contains no tool calls, atomically close the steering inbox if it is
   empty. Pending guidance instead joins the same turn and opens another
   provider request.
5. Otherwise preview each call, request approval when required, and execute it.
   Consecutive shared reads run in bounded parallel waves; exclusive calls
   remain ordered barriers. Collect every result in original call order inside
   one user message, then append any queued guidance.
6. Repeat until the model answers, the user interrupts, or a fatal error occurs.

Ordinary turns have no model-request ceiling. The controller accepts an
internal request budget for deterministic test fixtures; configuration and
TUI launch do not expose or pass that option.

The controller awaits a checkpoint after every complete tool-result batch and
after the final assistant response. It never begins the next provider request
until that checkpoint succeeds. The TUI commits the canonical turn to its
durable session owner first; ephemeral conversations commit only in memory.

Interactive steering is cooperative rather than preemptive. The composer feeds
one bounded inbox owned by the active turn. Guidance is consumed after the
provider response or complete assistant-issued tool batch already in progress,
never by cancelling or replaying part of that batch. It becomes an ordinary
user message in canonical and model-facing history and remains inside the same
conversation node. An atomic empty-and-close handshake prevents guidance from
arriving between final completion and persistence; interruption returns any
unconsumed guidance to the composer.

If an interactive provider turn fails or is interrupted, the TUI seals the
visible partial evidence and commits an explicit `failed` or `interrupted`
settlement. A neutral local assistant closure keeps the next provider request
structurally valid without pretending that partial streamed text was an
authoritative provider message. The same failure evidence is then materialized
by the live transcript, export, timeline, and resume.

Canonical history and provider context are separate arrays. The controller
always appends authoritative assistant messages and tool results to both, but a
context hook may replace only the provider-facing array. One definite 400/413
context-limit rejection can therefore be retried after compaction without
replaying an ambiguous generation failure or rewriting durable history.
Provider-facing tool evidence is bounded independently from canonical history.
Each ordinary result keeps an append-stable excerpt, so adding later evidence
does not rewrite the request prefix and defeat prompt caching. Semantic
compaction establishes the only normal boundary that may replace that prefix;
if compaction cannot recover a saturated request, a bounded newest-first
projection remains as the final safety fallback.

Before every provider request, the controller resolves the current model
context policy. Provider adapters cache stable metadata themselves and refresh
it when needed. The conservative request estimate includes the system prompt, projected
messages, tool schemas, a fixed wire-envelope allowance, entropy-sensitive
headroom, and an additional safety reserve. The configured `maxTokens` value is
an output ceiling: it is reduced when necessary so estimated input plus maximum
output stays inside the model window or a stricter provider safety limit. An
envelope that cannot leave a small useful response budget is rejected locally
before network generation begins. Compaction summary requests use the same
budget boundary. After a successful request, provider-reported input usage is
the preferred pressure signal; when it is absent, the conservative sent-input
estimate replaces stale pressure without being added to vendor usage totals.

A tool failure becomes an error result the model can act on. Cancellation is
the exception: it propagates through provider HTTP, retry waits, filesystem
searches, atomic file mutations until their rename commit, and process trees so
the foreground operation can end promptly.

## Foreground activity

The interactive shell owns one model turn and at most one command interaction,
each with its own `AbortController`. Commands may load catalogues or open menus
while the model streams. Tool approvals have a separate overlay slot and take
focus without discarding a command's picker, query, or field. Settling either
workflow closes only its own interaction. Input cancellation targets the visible
owner; process shutdown and `Ctrl+D` cancel both and reject new input.

The turn captures provider, model, effort, request identity, and context settings
before its first await. Continuations, compaction, errors, and checkpoints use
that snapshot; menu changes configure the next turn. The footer keeps the active
identity until settlement. Live permission revocation is rechecked before tool
preview, approval, and execution. Already-running tools are not retroactively
cancelled. `/new`, `/timeline`, and `/compact` require idle conversation state;
`/export` clones the visible transcript before asynchronous writing.

A fatal rendering or input callback failure cancels all open overlays, aborts
both workflows, and awaits their settlement before closing session
persistence. Only then can control return to the caller, so provider or tool
work cannot continue behind a restored terminal.

There is no startup banner, permanent preamble, or automatic menu. Every launch
opens on an empty transcript and composer; readiness remains visible in the
footer, and configuration opens only when the user invokes it. `/providers`
owns API keys and OpenAI Account OAuth. `/models` asks every
currently usable provider for its catalogue concurrently, keeps successful
catalogues when another provider fails, and presents one searchable list.
Choosing a row changes provider and model atomically; cancellation or a failed
settings write restores both. `/settings` is model-first and links back to the
same provider-access flow rather than duplicating it.

## Provider boundary

The controller uses the normalized types in `src/types.ts`: messages contain
text, tool calls, tool results, optional opaque provider data, and optional
usage. Providers translate this vocabulary to four transport/authentication paths:

| Provider | API | Deployment |
|---|---|---|
| Anthropic API | Messages | Cloud |
| OpenAI API | Responses | Cloud |
| OpenAI Account | ChatGPT Codex Responses | Cloud |
| Ollama API | OpenAI-compatible Chat Completions | Official cloud API |

Each provider has three responsibilities:

- `*-wire.ts` translates complete requests and responses.
- `*-stream.ts` assembles authoritative responses from events.
- `*.ts` applies authentication, model-specific fields, and HTTP transport.

Streamed text is display-only. The complete message returned by `send` is the
only assistant message appended to history. Anthropic thinking signatures and
OpenAI reasoning items are retained as provider-tagged opaque data so a later
request can echo them without corrupting or inventing fields. Ollama reasoning
is retained with the assistant tool call for compatible Chat Completions
continuations. Cross-provider history falls back to normalized content.

OpenAI Responses requests use `store: false` and request encrypted reasoning
content for stateless continuation. The `openai` provider authenticates with
`OPENAI_API_KEY`; the separate `openai-codex` provider uses an explicitly
connected ChatGPT account and the ChatGPT Codex backend. Their opaque history
is tagged separately so it is never replayed across those trust boundaries.
Model selection stores the provider and model together, while the footer keeps
that route visible. Connecting an account or adding a key never changes it.
OpenAI API requests carry a random client request identifier and bounded server
request metadata for support correlation without logging prompts or secrets.
Every controller session also derives one stable, non-secret request identity
for each provider from its durable conversation or process-local session state.
Retries and subsequent requests on that route reuse it for provider cache
affinity without coupling API-key traffic to ChatGPT account traffic.
Refusals, incomplete responses, nested failures, and usage are normalized
rather than disappearing at the stream boundary.
Truncated responses never authorize tool execution. Output-limit diagnostics
direct the user to maximum output tokens in `/settings` when that control
applies to the selected provider.
Malformed, scalar, and array tool arguments retain a transient invalid marker
through dispatch. The controller returns a matching error result without
previewing or executing the call; durable codecs omit the marker after that
result makes the failure explicit.

The shared HTTP client owns bounded transport but not provider semantics.
Adapters normalize authentication, billing, quota, rate-limit, overload,
context, network, and unknown failures. Idempotent catalogue GETs may retry
transient network, rate-limit, and 5xx failures. A streaming generation gets one
retry only when its adapter classifies a pre-stream rejection as a transient
rate limit and the provider supplies an explicit delay. Billing and quota stops
are never retried. Jecode caps a retry wait at 60 seconds and keeps it visible
and interruptible.
Generation POSTs are never replayed after an ambiguous network error, a server
failure, or any stream progress. A generation reports whether it is connecting
or waiting for the model. A request has 60 seconds to receive response headers,
and an open JSON or SSE body can remain idle for at most 120 seconds. OpenAI
Responses streams also have five minutes to produce substantive model progress;
protocol keepalives do not extend that deadline.
Process signals cancel the active provider or tool before Jecode exits, with a
bounded hard-exit fallback. The client handles redirects manually
and rejects every 3xx response without retrying or forwarding headers to
another endpoint. Retry state is surfaced in the TUI. Each SSE event, the
model-output-aware aggregate stream, reconstructed tool arguments, model
catalogue, and per-response tool-call batch have explicit limits; overflow cancels
or rejects the response before unbounded work reaches the controller.

Ollama uses the fixed `https://ollama.com` API and requires `OLLAMA_API_KEY`
for catalogue, metadata, and generation requests. Its access menu uses the same
key flow as the other API providers. `ollama-context.ts` reads cloud model
capacity from `/api/show`, caches it briefly, and applies safety headroom;
there is no local daemon discovery, allocation probe, or custom endpoint state.
`ollama-endpoint.ts` owns the fixed origin and recognizes retired cloud settings
only for compatibility. Startup rejects other legacy endpoint values before
any provider request, and settings writes preserve those files until the user
explicitly removes the retired field.

## Workspace boundary

`src/tools/index.ts` registers the workspace capabilities. `file-read.ts` owns
bounded reads and directory listings; `file-write.ts` keeps mutation previews,
replacement rules, and atomic revalidation together. `search.ts` owns bounded
discovery and text scanning, using the pure matcher in `glob.ts`. Both paths
share the filesystem boundaries in `paths.ts` and the stable file readers.

Read and write tools accept paths inside one configured root. Lexical checks
reject traversal and outside absolute paths. Existing paths are then resolved
with `realpath`, and missing write targets resolve through their nearest
existing parent. This closes symlink and Windows junction escapes.

Reads may follow an alias only when its canonical target remains inside the
workspace. Writes and edits require a direct path: every symlink or junction
component is rejected. The boundary is revalidated after the temporary sibling
is opened but before content is written, and again immediately before rename;
the temporary pathname must still identify the file Jecode opened. A write or
edit snapshots the target at execution time, verifies any previewed state, and
compares that snapshot again in the final before-rename validation. This also
distinguishes an absent target from an existing empty file, so an intervening
change is rejected instead of overwritten. Node exposes no cross-platform
compare-and-swap replacement primitive, so another process can still write in
the irreducible interval between that final check and the rename.

File reads and whole-file mutations accept regular files only. Reads use
bounded, cancellable handles that cannot wait on FIFOs or other special files.
Identity-sensitive metadata remains bigint, and discovery hands the exact file
generation to the descriptor reader. A replacement between discovery and open
is rejected rather than searched.
Mutations enforce independent byte, character, and line budgets; existing
content is read through a bounded file handle, and `replace_all` checks its
projected size before allocating the result. Files above that budget remain
available through ranged `read_file` calls, but `write_file` and `edit_file`
will not replace them wholesale.

The built-in tools are:

| Tool | Default | Execution | Boundary |
|---|---|---|---|
| `read_file` | Allow | Shared | Progressive UTF-8 read, bounded output, canonical path |
| `list_dir` | Allow | Shared | Bounded entries and output from one canonical directory |
| `find_files` | Allow | Shared | Bounded recursive glob; skips VCS, dependencies, symlinks |
| `search_text` | Allow | Shared | Bounded literal search over verified file generations |
| `edit_file` | Ask | Exclusive | Bounded exact replacement, atomic write, preview check |
| `write_file` | Ask | Exclusive | Bounded whole-file replacement, atomic write, preview check |
| `run_command` | Ask | Exclusive | Workspace working directory, timeout, bounded output |

`search_text` enumerates candidates through Jecode's bounded workspace walk and
captures each regular file's identity while its parent directory is stable.
The owned scanner opens those candidates in bounded parallel batches, checks
the captured identity and workspace boundary throughout the read, and retains
canonical file order. It never delegates workspace reads to an executable from
`PATH`.

`run_command` is not a filesystem sandbox. A shell can address anything the
user account can address, which is why every exact command asks by default
unless its session policy or a remembered approval allows it. On cancellation or
timeout, jecode terminates the process tree and escalates if it does not exit.
The child receives an explicit copy of the process environment with
credential-like names removed. Known environment, session, and saved
credential values are redacted from captured output before the controller can
send it to a provider or retain it in a transcript block. While the process is
running, the same bounded and redacted capture is emitted as replaceable TUI
snapshots; raw chunks never cross the tool boundary. The redaction snapshot
re-reads canonical credential stores at the command boundary, so a token
rotated by another Jecode process is covered alongside the cached value.

The “allow this session” choice is deliberately narrow: one target file for
file changes, or one exact shell command. `/permissions` exposes every tool in
one session-only control plane. Read-only tools can be allowed or denied;
dangerous tools can ask, allow, or deny. Left/Right changes the selected policy
without opening another menu, while Enter opens remembered approvals only when
that tool has any. Denied tools are omitted from the next model turn's catalogue;
calls already advertised in the current turn are denied before execution, and
changing a tool policy clears its remembered grants. Every policy remains
adjustable, and `/new` restores the defaults and clears remembered approvals.

## Settings, credentials, and local data

Persistent user data lives under `~/.jecode`, outside every workspace.
`settings.json` contains only non-secret defaults: provider, one remembered
model per provider, effort, output and compaction limits, and reduced motion.
Provider and model selection, effort, output, and compaction use saved settings
or built-in defaults; `/models`, `/effort`, and `/settings` own their changes.
Root and ephemeral mode stay process-only. Reduced motion retains flag,
environment, saved setting, then default precedence, while ephemeral mode uses
its flag, environment, then default. The remaining preference environment
variables are `JECODE_REDUCED_MOTION` and `JECODE_EPHEMERAL`.

`config.ts` rejects retired model-setting, request-budget, and auto-approval
flags with removal guidance. Their nonempty legacy environment overrides also
stop startup; diagnostics report only the variable name and applicable TUI
control. This startup cleanup changes no saved schema and migrates no data.

API keys resolve from environment, then in-memory session values, then the
saved `~/.jecode/credentials.json` file. The environment wins. ChatGPT OAuth is
an independent authentication kind stored in `~/.jecode/accounts.json`; it
never shadows an API key or turns the OpenAI API provider into a subscription
provider. The TUI never prints a secret, and authentication values never enter
message history or transcript blocks. Older config-directory API keys remain a
read-only fallback until the canonical file is first written.

All three JSON stores are accepted only through regular-file handles, opened
non-blocking where the operating system supports it. Size caps are enforced
before content is read or parsed: 64 KiB for settings, 256 KiB for saved API
keys, and 128 KiB for OAuth accounts. The same caps apply before writes, and
known fields also have count and string limits. Malformed, oversized, or
out-of-schema state falls back without becoming startup work proportional to
attacker-controlled input. Startup remains tolerant and treats damaged state as
unconfigured; a mutation instead rejects unknown, malformed, or oversized state
without rewriting it. Credential redaction uses a bounded secret set and fails
closed if that supported set is exceeded.

Saving is explicit. Secret files use owner-only modes on POSIX; Windows relies
on the user profile ACL. Store directories are captured as canonical direct
anchors, so an alias retarget cannot redirect a later lock or write. Replacement
uses the same atomic writer as workspace files; POSIX attempts a
parent-directory sync after rename. Every persistent mutation serializes
through a bounded cross-process lock and rereads the store inside it, so
concurrent Jecode sessions preserve unrelated settings, keys, and rotated
tokens. Age alone never authorizes stale
lock recovery: a live owner keeps the lock through a slow network refresh, and
an abandoned lock is removed owner-first so competing waiters cannot steal a
newly acquired lock. `/providers` shows only API-key sources or a non-secret
ChatGPT identity and plan hint. OAuth uses PKCE for
browser login, supports OpenAI's device-code path for WSL/headless terminals,
refreshes early, and retries one 401 after refresh. Logout removes the local
account even when remote revocation cannot be confirmed.

The loopback callback serves the self-contained result page from
`oauth-result-page.ts`: a small Jeco identity, one outcome, and a return-to-terminal
instruction. Success and failure share a static layout. The page loads no external
assets and removes the authorization query from browser history; protocol checks
and response security headers remain in `openai-oauth-callback.ts`.
`dev/web/` previews the same renderer without authenticating or reading account data.

`/export` is an explicit, argument-free operation that writes an automatically
named Markdown transcript in the directory from which Jecode was launched. It
requires no picker or approval. `/new` closes the current durable conversation,
then clears history, usage, transcript, and session approvals.

## Conversation state and resume

`ConversationTree` is the single semantic source for complete normalized
history and the settled TUI transcript. One node owns one user turn and its
model/tool messages, visible transcript blocks, provider/model/effort identity,
revision, and settlement. The selected root-to-leaf path is materialized in
full for the screen, export, and audit. A separate `contextHistory` projection
is materialized for providers. `/timeline` projects every resumable node --
completed, failed, or interrupted -- into a searchable compact tree and can
select any of them without changing durable state. The next real user turn
appends from that selected parent and advances the existing session head;
cancelling or exiting before then creates no node. The settled transcript and
full history are materialized again from the newly selected path, while
historical tool records remain inert.

Context compaction is automatic and model-aware. Pressure combines the most
recent normalized input-token count with a conservative byte estimate. The
provider boundary can advertise one model's usable request window and a stricter
automatic-compaction ceiling: OpenAI Account retains those fields from its live model
catalogue, Anthropic retains `max_input_tokens`, and Ollama reads cloud model
capacity from `/api/show`. The OpenAI
API model list does not include capacities, so its accepted reasoning families
use isolated conservative metadata. Discovery failures use a 200k fallback and
therefore leave ordinary turns available; an envelope that already exceeds that
fallback is rejected locally instead of sending an unbounded request.

Request estimation preserves the same provider-neutral byte, compression, and
literal floors while processing serialized input in bounded chunks. Large
estimates yield between chunks, observe cancellation, and are reused by
automatic or overflow compaction for the exact projection already measured.
The planner checks only a logarithmic set of safe recent-turn boundaries and
does not re-estimate the selected tail. Projection changes always receive a
fresh request estimate before the provider is called.

The saved `compactionPercent` setting accepts 50 through 95 and defaults to 85.
It applies to the usable model window while a lower provider safety ceiling and
the estimator's safety reserve still win. The post-compaction target is one
quarter of that window and the
recent exact tail is bounded to half that target; both therefore scale from a
small context windows to million-token models. Capacity discovery runs before
each request; provider adapters retain safe metadata where appropriate and
refresh it periodically. The selected provider receives one
streamed, tool-free summary request with a neutral instruction that treats all
source content as untrusted data. Opaque provider blocks are removed from that
request. The resulting user-level summary never enters the system prompt and is
never rendered as transcript content.

A context anchor records the summary and its exact node/message boundary. The
anchor lives on the active branch, so projection uses the newest anchor on the
selected path and timeline branching can choose another one later. The
canonical prefix remains untouched. TUI checkpoints persist the ordinary turn
before attempting optional compaction, then atomically revise that same leaf if
a summary succeeds. Cancellation or failure therefore cannot discard a
completed response or a settled tool batch. `/compact` forces that same policy
below the automatic trigger and atomically revises only the active leaf with a
newer anchor. It is a silent no-op when there is too little useful prefix. A
temporary historical selection
must receive a new user turn first, because compacting the shared branch point
would rewrite history owned by more than one path.

Interactive sessions publish lazily after the first consistent tool
checkpoint or settled completed, failed, or interrupted turn. They live under
`~/.jecode/sessions/<workspace-digest>/<session-id>/` with versioned metadata,
one atomically replaced file per conversation node, a small atomic head, and a
rebuildable 4 KiB catalogue summary tied exactly to that metadata and head. The
summary has its own version; it does not change the canonical session schema.
The node lands before the head. After a crash, the loader accepts only one
strictly adjacent node mutation and advances the head; any ambiguous state is
rejected. Files and decoded values are size-bounded, unknown fields fail
closed, and persisted provider messages drop opaque `raw` data before crossing
the disk boundary. The encoder applies the same per-field validation before a
node can enter the tree or replace a file. Schema 2 added bounded context
anchors; schema 3 added durable failed/interrupted outcomes; schema 4 adds
settled tool durations. The strict decoder continues to accept every older
schema and upgrades the active node on its next checkpoint.

Under `src/sessions/`, `store.ts` retains publication, checkpoint, and ownership
transactions. `bucket.ts` holds the workspace identity and directory anchors;
`files.ts` handles bounded node IO; `load.ts` validates the complete tree and
adjacent recovery; `catalog-io.ts` lists and repairs catalogue summaries.
`snapshot.ts` validates the in-memory checkpoint boundary. The file envelopes
and schema compatibility remain in `codec.ts`, with message and transcript
codecs in `codec-messages.ts` and `codec-transcript.ts`, sharing bounded value
validation through `codec-values.ts`.

The interactive persistence owner retains the last fully verified snapshot and
its exact store-scoped lease. Conversation nodes are deeply immutable, so a
checkpoint can verify
shared history by identity, update aggregate size counters for only the changed
node, reread the bounded head and lease, then write the candidate catalogue,
one node, and finally the head. The candidate catalogue matches only the new
head, so a partial checkpoint cannot leave an old summary looking current.
A bounded checkpoint marker prevents concurrent recovery until the writer
releases ownership. A changed head, replaced lease, rematerialized
history, or unexpected node outside that snapshot fails closed. Node reads use
bounded parallel batches with per-file, in-flight, and aggregate byte caps.
Resume always reads and validates the complete on-disk tree. Recovery validates
the candidate tree and catalogue before advancing the durable head; invalid
conversation data leaves that head unchanged. The persisted conversation schema
is unchanged.

Session catalogues use the canonical real workspace path, so another project
cannot appear in the resume picker. Generation-specific process leases prevent
simultaneous resume of the same logical session. Its directory and identifier
remain stable across exits and resumes; each resumable turn advances the head inside that
session's tree, while `/new` or a fresh launch creates another session. A
failed or user-interrupted turn resumes exactly with its partial visible
evidence and explicit outcome. An abrupt process stop inside a tool loop leaves
only a `checkpointed` node and therefore resumes from its latest safe ancestor;
a first turn with no resumable ancestor is not offered. The next completed turn
branches from that ancestor without creating a second catalogue entry. This
keeps Anthropic thinking signatures and equivalent provider continuation state
out of durable storage without constructing or replaying a partial tool turn.
No load, resume, or branch operation executes a historical tool call.

The resume catalogue validates bounded metadata, head, summary, marker, and
lease files for every entry in its 4,096-entry input set. It rereads each head
to reject a mixed checkpoint view, processes independent sessions in small
concurrent waves, and sorts the whole admissible set by durable `updatedAt`
before applying the visible result limit. A missing, malformed, head-stale, or
abandoned-checkpoint summary triggers the strict full loader while the session
is idle, then is rebuilt atomically. Version 1 indexes also take this path once:
their writer could leave a current-looking summary hiding an adjacent node
after a failed head write. Rebuilt indexes use version 2; canonical conversation
schemas stay unchanged. Selecting a row always performs the strict
load again, so the summary never authorizes conversation data. The catalogue
fails explicitly above its entry bound instead of silently hiding an older
session that was updated most recently.

`jecode resume` opens a searchable selector; `jecode -c` and its equivalent
`jecode resume --last` choose the newest available source through the same launch
path. `--ephemeral` omits the persistence owner entirely. Draft editor content,
ordinary transient notices, permission policies, approvals, credentials, and
pending UI blocks are never part of a checkpoint.

## TUI composition

The current visual direction and its working examples live in
[`dev/tui/DIRECTION.md`](../dev/tui/DIRECTION.md) and the
[TUI lab](../dev/tui/README.md). This section describes runtime composition and
state ownership. Accessibility and frame-safety guarantees remain in
[terminal compatibility](COMPATIBILITY.md#terminal-interaction).

The TUI owns the alternate screen and speaks terminal protocols directly:
raw input, bracketed paste, SGR mouse reports, synchronized frame output, and
cursor style. `screen.ts` is the terminal lifecycle boundary. `app.ts` owns
mutable application state and schedules paints; rendering returns rows and a
cursor without writing to the terminal.

```text
state + terminal size
  → view.ts
  → transcript-view.ts / modal.ts
  → blocks.ts
  → tui/components/*
  → ui/render.ts
  → rows + cursor
  → frame.ts diff
```

The painter checks asynchronous output readiness before encoding a frame. While
the stream needs a drain, input and semantic state continue to advance without
queuing intermediate frames. A drain schedules a fresh composition against the
last accepted frame. Both the application and lab remove their drain listener
before returning the terminal; synchronous OS writes remain outside this control.

Each visible section has one renderer under `src/tui/components/`;
`blocks.ts` routes semantic transcript blocks to them at the full terminal width.
Model prose and tool records share the composer's left edge; user surfaces keep
their own content padding. `tool.ts` composes target-first
headers and connectors; `tool-evidence.ts` selects bounded previews and renders
retained source, and `tool-motion.ts` colours only the active connector.
The lower dock supplies
one shell for the editor, completion, selectors, searchable queries, credential
fields, and help. Shared prompt and menu-row renderers own cell measurement,
selection, secret masking, progress, and caret placement. `picker.ts` owns
filtering and selection; `picker-layout.ts` shares one row budget between
painting and caret measurement. `components/menu.ts` supplies Ribbon rows and
bounded overflow recovery for clipped labels and values to selectors and command
completion. Timeline previews opt into in-row truncation, so long turns never
reserve overflow rows or repeat the selected preview below the list.
Options carry no explanatory description. Model catalogue failures
use footer feedback instead of a persistent picker preamble. Semantic `Palette`
tokens keep colour roles out of component implementations.

Clearing the transcript releases every retained width and active-tool reference,
so `/new` cannot keep the previous conversation alive through a resize cache.
Blocks store source text, never pre-wrapped rows. A transcript renderer caches
rows per block, width, and palette: streaming invalidates the changing block,
scrolling reuses cached rows, and resize reflows them. It assembles only the row
ranges intersecting the viewport and bounds background reflow work. Source
text, tool evidence, and settled execution duration remain available to
expansion, resume, and export even when their visible previews are compact.

Tool animation uses the running call's start time and a set of active blocks,
without per-row arrival or settlement histories. Only visible, collapsed,
running records request animation frames. Expansion, scrolling away from the
live tail, `NO_COLOR`, and reduced motion suppress decorative movement;
clock-driven activity paints still refresh elapsed time. Waiting and historical
calls neither animate nor replay.
The viewport is bottom-relative: offset zero follows output; growth below a
scrolled viewport increases its offset to preserve the reading position.
Tiny terminals receive a fixed recovery frame instead of overflowing chrome.

Terminal cell measurement, truncation, styled spans, and editor movement share
grapheme boundaries across combining marks, emoji, and wide CJK glyphs.
Untrusted text is neutralized before measurement and paint: control sequences,
ESC/CSI/OSC, delete, and bidirectional controls remain visible data. Renderer
styling escapes are introduced only after that boundary. `NO_COLOR` preserves
structural focus and state marks; reduced motion also makes the cursor steady.

The input path is `keys.ts` → `app-input.ts` → `input.ts`, `editor.ts`,
`complete.ts`, or `overlay.ts`. The decoder converts terminal-specific bytes
into semantic actions. Word movement and deletion use whitespace-delimited
words; Windows Terminal's BS/DEL distinction applies only when `WT_SESSION`
identifies it. Generic terminals retain traditional Backspace encodings.

Prompt ingress shares the session text boundary: 1,048,576 UTF-16 code units.
The decoder bounds paste and protocol buffers before concatenation, and the
editor refuses insertions that cross the same limit. An overflow produces
footer feedback and prevents submission until the prompt changes. Cooperative
steering has its own smaller per-turn queue.

`app-input.ts` checks readiness before clearing the editor or appending
history. During a turn, slash commands retain their command semantics while
ordinary text offers guidance to the existing steering inbox. Neither creates
another model turn. `app-workflows.ts` connects the shell to
`command-workflow.ts` and `turn-workflow.ts` with one shared automatic-compaction
gate. The turn workflow retains steering, checkpoint order, and failure recovery
as one transaction. `turn.ts` translates controller events into semantic
transcript blocks; `tool-details.ts` constructs their targets, output, and diff
details. Slash commands can emit transient feedback, open a dock
interaction, or request a lifecycle action; their types cannot create semantic
transcript blocks.

Operational state and feedback occupy the footer. Warnings and errors take
priority over active state, followed by informational feedback and unseen
output. Replacement and expiry belong to `feedback.ts`; those messages never
enter the conversation, checkpoint, or Markdown export.

The development lab imports these production components and input handlers.
Its host supplies inert callbacks and deterministic fixture time, while the
release runtime has no dependency on `dev/`. Real persistence, provider access,
and controller behavior remain covered by their own integration tests.

## Validation boundaries

Pure translation, layout, width, activity, scroll, permission, and transcript
logic is unit tested. Integration tests cover the packaged command, bootstrap
routing and terminal rejection, durable session recovery, context projection and
compaction, TUI screen ownership and restoration, stream assembly, HTTP retry
and cancellation, provider request bodies, tool-loop semantics,
symlink/junction confinement, stable file generations, generation-safe leases,
atomic preview checks, shell process-tree
termination, credential precedence, and bounded search.

Suites under `test/` follow those behavior boundaries: controller scheduling,
steering and recovery, provider wire and stream handling, file reads and
mutations, and distinct TUI workflows. Shared inert factories and host harnesses
live under `dev/test-support/`; each suite owns its setup and cleanup. Keeping
helpers outside `test/` prevents Node's default discovery from treating helper
modules as standalone test files.

[`npm run check`](../scripts/README.md) is the canonical automated gate. Its
shared compiler rules cover runtime source, development tools, scripts, and
tests; release builds inherit the same rules. Coverage, source-tree, package,
and installed-CLI checks run through the documented scripts.

Automated checks do not establish live provider behavior, physical terminal
accessibility, or long-session reliability. The
[candidate validation protocol](../dev/validation/README.md) defines those
checks and their evidence; the [compatibility contract](COMPATIBILITY.md#release-candidate-gate)
owns promotion requirements.

## Deliberate omissions

- No runtime dependency, SDK, curses layer, install-time compiler, or bundler.
  TypeScript remains the development source; the ignored release-only `dist/`
  tree is emitted by the existing development compiler before packing. Registry
  users receive that JavaScript runtime and execute no installation scripts;
  Git dependency installs are intentionally unsupported.
- No branch rename, deletion, or merge surface. Timeline navigation is append
  only: existing nodes and alternate paths remain available for inspection.
- No fixed token threshold, provider-native history mutation, or second agent
  loop. One bounded percentage controls automatic compaction, while `/compact`
  invokes the same model-aware policy and the durable tree remains complete.
