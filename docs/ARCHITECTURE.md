# Architecture

## Mental model

jecode has one agent loop. The TUI or batch reader supplies a user message;
the controller sends it to one provider, executes requested tools, and repeats
until the model returns no tool calls.

```text
bin/jecode.js → dist/main.js → start.js
                               ├─ tui/app.js       interactive control plane
                               │    ├─ app-input.js
                               │    └─ app-workflows.js ─ commands.js
                               └─ batch.js         piped control plane
                                        │
                                        ▼
                                   controller.js   the only agent loop
                                     ├─ providers/ wire-format boundaries
                                     └─ tools/     workspace capabilities

npm run start → src/main.ts    direct development path
npm run pack:release → clean build → dist/ inside the release tarball
```

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
6. Repeat until the model answers, the user interrupts, a fatal error occurs,
   or an explicit process-only `--max-steps` budget is exhausted.

The controller awaits a checkpoint after every complete tool-result batch and
after the final assistant response. It never begins the next provider request
until that checkpoint succeeds. Batch mode commits only to memory; the TUI
commits the same canonical turn to its durable session owner first.

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

Before every provider request, the controller resolves the current model
context policy. Provider adapters cache stable metadata themselves, while this
boundary lets Ollama observe a smaller runtime allocation after loading a
model. Its conservative request estimate includes the system prompt, projected
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

The interactive shell permits one foreground activity: a slash command or a
model turn. One `AbortController` owns its cancellation. The composer may feed
cooperative guidance into that model turn without opening another activity. This
prevents a network-backed `/models` request from overlapping a turn and gives
`Esc`, `Ctrl+C`, and `Ctrl+D` one consistent target.

A fatal rendering or input callback failure cancels any open overlay, aborts
that foreground activity, and awaits its settlement before closing session
persistence. Only then can control return to the caller, so provider or tool
work cannot continue behind a restored terminal.

There is no startup banner, permanent preamble, or automatic menu. Every launch
opens on an empty transcript and composer; readiness remains visible in the
footer, and configuration opens only when the user invokes it. `/providers`
owns API keys, ChatGPT OAuth, and Ollama connections. `/models` asks every
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
| Anthropic | Messages | Cloud |
| OpenAI | Responses | Cloud |
| OpenAI Codex | ChatGPT Codex Responses | Cloud |
| Ollama | OpenAI-compatible Chat Completions | Local loopback or hosted |

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
Refusals, incomplete responses, nested failures, and usage are normalized
rather than disappearing at the stream boundary.
Malformed, scalar, and array tool arguments retain a transient invalid marker
through dispatch. The controller returns a matching error result without
previewing or executing the call; durable codecs omit the marker after that
result makes the failure explicit.

The shared HTTP client retries transient network errors, rate limits, and 5xx
responses for idempotent catalogue GETs. A streaming generation gets one retry
only when the provider explicitly rejects it with `429` and supplies a retry
delay before a stream begins. Jecode caps that wait at 60 seconds and keeps it
visible and interruptible.
Generation POSTs are never replayed after an ambiguous network error, a server
failure, or any stream progress. A generation reports whether it is connecting
or waiting for the model. A request has 60 seconds to receive response headers,
and an open JSON or SSE body can remain idle for at most 120 seconds. OpenAI
Responses streams also have five minutes to produce substantive model progress;
protocol keepalives do not extend that deadline.
These internal deadlines also cover batch mode. Process
signals cancel the active provider or tool in either surface before Jecode
exits, with a bounded hard-exit fallback. The client handles redirects manually
and rejects every 3xx response without retrying or forwarding headers to
another endpoint. Retry state is surfaced in the TUI. Each SSE event, the
model-output-aware aggregate stream, reconstructed tool arguments, model
catalogue, and per-response tool-call batch have explicit limits; overflow cancels
or rejects the response before unbounded work reaches the controller.

Ollama initializes its endpoint from `--ollama-host`, `OLLAMA_HOST`, saved
settings, then key-aware inference. A configured API key selects
`https://ollama.com`; without one, Jecode targets the local daemon at
`http://127.0.0.1:11434`. The Ollama connection flow in `/providers` can replace
that session value live with Cloud, local, or a custom endpoint. HTTP is
accepted only for exact loopback hosts. Remote endpoints must use HTTPS, and
credentials embedded in the URL are rejected.

## Workspace boundary

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
| `search_text` | Allow | Shared | Bounded literal search; optional `rg`, built-in fallback |
| `edit_file` | Ask | Exclusive | Bounded exact replacement, atomic write, preview check |
| `write_file` | Ask | Exclusive | Bounded whole-file replacement, atomic write, preview check |
| `run_command` | Ask | Exclusive | Workspace working directory, timeout, bounded output |

`search_text` enumerates and validates candidates through Jecode's own bounded
workspace walk. For larger candidate sets, an optional `rg` from `PATH`
accelerates literal matching over those files with a credential-filtered
environment. Jecode resolves the executable to a canonical absolute path and
rejects candidates controlled by the workspace, so a repository cannot shadow
the helper. Smaller searches and machines without a trusted `rg` use the
dependency-free TypeScript scanner. Candidate validation and portable reads run
in bounded batches, while their results retain canonical file order. `rg` is
never required at install or runtime. Because ripgrep's own match limit applies
per file, Jecode stops and rejects an accelerated batch as soon as its raw
events exceed the requested global result limit; the portable scanner then
produces the bounded answer.

`run_command` is not a filesystem sandbox. A shell can address anything the
user account can address, which is why every exact command asks by default
unless its session policy allows it or the process started with
`--auto-approve`. On cancellation or
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
that tool has any. Denied tools are omitted from the next model request, and
changing a tool policy clears its remembered grants. `/new` restores the
defaults. A launch-time `--auto-approve` keeps dangerous tools locked to allow
for that process.

## Settings, credentials, and local data

Persistent user data lives under `~/.jecode`, outside every workspace.
`settings.json` contains only non-secret defaults: provider, one remembered
model per provider, the Ollama endpoint, effort, output and compaction limits,
and reduced motion. Runtime precedence is CLI flags, environment variables,
saved settings, then built-in defaults. Root, auto-approval, and the optional
model-request budget stay process-only.

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
attacker-controlled input. Credential redaction uses a bounded secret set and
fails closed if that supported set is exceeded.

Saving is explicit. Secret files use owner-only modes on POSIX; Windows relies
on the user profile ACL. Replacement uses the same atomic writer as workspace
files. Every persistent mutation serializes through a bounded cross-process
lock and rereads the store inside it, so concurrent Jecode sessions preserve
unrelated settings, keys, and rotated tokens. Age alone never authorizes stale
lock recovery: a live owner keeps the lock through a slow network refresh, and
an abandoned lock is removed owner-first so competing waiters cannot steal a
newly acquired lock. `/providers` shows only API-key sources or a non-secret
ChatGPT identity and plan hint. OAuth uses PKCE for
browser login, supports OpenAI's device-code path for WSL/headless terminals,
refreshes early, and retries one 401 after refresh. Logout removes the local
account even when remote revocation cannot be confirmed.

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
automatic-compaction ceiling: ChatGPT retains those fields from its live model
catalogue, Anthropic retains `max_input_tokens`, and Ollama prefers the context
currently allocated by `/api/ps` before falling back to `/api/show`. The OpenAI
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
small Ollama allocation to million-token models. Capacity discovery runs before
each request; provider adapters retain safe metadata where appropriate, while
Ollama can replace theoretical capacity with its runtime allocation and refresh
that observation periodically. The selected provider receives one
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
completed response or a settled tool batch. Batch mode applies the same policy
in memory. `/compact` forces that same policy below the automatic trigger and
atomically revises only the active leaf with a newer anchor. It is a silent
no-op when there is too little useful prefix. A temporary historical selection
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

The interactive persistence owner retains the last fully verified snapshot and
its lease. Conversation nodes are deeply immutable, so a checkpoint can verify
shared history by identity, update aggregate size counters for only the changed
node, reread the bounded head and lease, then write one node followed by the
head and catalogue summary. A bounded checkpoint marker precedes the node
mutation, so a crash cannot leave an old summary looking current while an
adjacent node awaits recovery. A changed head, replaced lease, rematerialized
history, or unexpected node outside that snapshot fails closed. Resume always
reads and validates the complete on-disk tree; the persisted conversation
schema is unchanged.

Session catalogues use the canonical real workspace path, so another project
cannot appear in the resume picker. A process lease prevents simultaneous
resume of the same logical session. Its directory and identifier remain stable
across exits and resumes; each resumable turn advances the head inside that
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
is idle, then is rebuilt atomically. Selecting a row always performs the strict
load again, so the summary never authorizes conversation data. The catalogue
fails explicitly above its entry bound instead of silently hiding an older
session that was updated most recently.

`jecode resume` opens a searchable selector; `jecode resume --latest` chooses
the newest available source. `--ephemeral` omits the persistence owner entirely.
Batch mode is always stateless. Draft editor content, ordinary transient notices,
permission policies, approvals, credentials, and pending UI blocks are never
part of a checkpoint.

## TUI composition

The TUI owns the alternate screen and speaks terminal protocols directly:
raw input, bracketed paste, SGR mouse reports, synchronized frame output, and
cursor style. `screen.ts` is the only terminal lifecycle boundary.

The rendering path is intentionally pure at its boundary:

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

Blocks store source text, never pre-wrapped rows. A transcript renderer caches
rows per block, width, and palette: streaming invalidates only the changing
block, scrolling reuses all cached rows, and resize deliberately reflows them.
Ephemeral tool motion invalidates only its active block on an adaptive frame
clock; it is neither stored in the transcript nor replayed after resume.
The viewport is assembled from only the cached row ranges it intersects rather
than flattening the complete history on every frame. Cell measurement,
truncation, and styled spans share complete grapheme boundaries across
combining marks, emoji, and wide CJK glyphs. Markdown prose uses a readable
maximum measure while code and tables retain the available width.

Untrusted text is neutralized before both cell measurement and paint. C0, C1,
ESC/CSI/OSC, delete, bidirectional controls, and pasted control sequences
therefore remain visible data instead of terminal instructions. Renderer-owned
styling escapes are introduced only after this boundary.

The visual grammar follows a transcript rather than a dashboard or execution
diagram:

```text
user input       full-width subtle surface with `❯` in the semantic gutter
reasoning        unframed, muted, unlabeled three-row tail
assistant        unframed Markdown on the shared content column
tool activity    state mark plus continuous evidence rail beneath each call
selection        bold Slate label/value; arrow fallback without colour
composer         editor, mid-turn steering, query fields, and contextual menu inside one pair of rules
footer           identity left, feedback or active state with elapsed time right
```

Operational feedback is not conversation. Slash-command confirmations,
configuration guidance, activity state, and preflight blockers share the
replaceable right side of the one-line footer. Foreground work is summarized as
`state · elapsed · enter to steer · esc to interrupt`, replacing the steering
hint with a queue count when needed and adding no spinner or icon. Errors
and warnings take priority over that summary, followed by informational feedback
and unseen output. Informational messages expire; warnings and errors remain
briefly or until the next key. They never become transcript blocks and
therefore never enter Markdown exports. Turn readiness is checked before the
editor, recall history, or model history is mutated, so a missing key or model
leaves the unsent prompt in place.

The command boundary enforces that separation in its types: commands can emit
only transient notices, open a dock interaction, or request a lifecycle action.
They cannot create semantic transcript blocks. `/help` is a read-only keyboard
reference inside the dock and closes with Esc; command discovery remains on the
`/` completion surface. Token accounting stays internal instead of becoming a
persistent `/usage` report.

Conversation turns use one blank terminal row as their outer separator. A user
turn owns a full-width subtle surface whose cue aligns with the shared content
column. Assistant and reasoning rows remain unframed. Only tool rows form an
evidence rail: consecutive tools join without repeated gaps, while reasoning
breaks that rail and keeps its own unframed rhythm.

Short transcripts are bottom-aligned against the dock with one fixed blank row
before the composer's upper rule. Unused terminal height stays above the
conversation, so the distance between the newest output and composer does not
change as the transcript grows or footer status changes.

Each visible section has one renderer under `src/tui/components/`; `blocks.ts`
only routes semantic transcript blocks to them. The TUI lab imports the same
production frame composer, so its fixtures cannot drift into a parallel mock.
Jecode owns the component contracts, design tokens, interaction rules, and
every emitted terminal cell.

The lower dock has one shell. The normal editor, slash-command autocomplete,
selectors, searchable queries, credential fields, and the temporary keyboard
reference provide its inner rows; none draws its own border. Autocomplete opens
on `/`, keeps selection separate from the typed prefix, and puts window progress
on that input row. Searchable pickers use the same `→` input and a real caret.
Up/Down select, Tab completes, Enter runs, and Esc closes. Key legends are not
repeated inside every picker. Direct control surfaces omit ornamental titles
and descriptions; row labels and values carry the state. Adjustable rows keep
their current value on the right and use Left/Right without closing the picker.
The footer remains outside the shell, so it never jumps when a menu opens.

The input decoder normalizes terminal-specific bytes and escape sequences into
semantic editor actions before they reach the composer. Character movement and
deletion remain grapheme-safe; Ctrl+Left/Right moves by whitespace-delimited
word, and Ctrl+Backspace/Delete removes the adjacent word. Windows Terminal's
distinct BS/DEL pair is enabled only when `WT_SESSION` identifies that terminal;
generic terminals retain both traditional plain-Backspace encodings.

Prompt ingress shares the session text boundary: 1,048,576 UTF-16 code units.
The raw-key decoder bounds bracketed-paste and protocol buffers before
concatenation, while the editor refuses an insertion that would cross the same
limit. An overflow stays in the footer and makes the retained prompt
unsubmittable until the user edits it. Batch stdin is split incrementally at
the same boundary and fails before echo, history, or provider use.
Cooperative steering retains its smaller, separate per-turn queue boundary.

Writable fields and searchable pickers carry the same `→ ` active prompt. One
shared renderer owns its terminal-cell width, horizontal window, secret mask,
right-side progress, and cursor offset, so the interactions cannot drift apart.
Selection colours only the active label and value in bold Slate, leaving the
row background untouched. `NO_COLOR` restores the arrow marker so focus remains
structural when colour is unavailable.

Assistant Markdown follows the same restraint. Bright foreground carries prose;
a dedicated technical cyan identifies paths, links, inline code, list marks,
and syntax keywords without spending the structural Slate accent. Readable
secondary text and deliberately dim chrome are separate roles, so explanations
remain legible without making reasoning, fences, or footer metadata compete
with the answer. Inline code has no background chip. Fenced code keeps dim
opening and closing fences with a two-cell body indent; it has no full-width
surface or decorative left rail.

Tool output and diffs stay complete in state. A running tool owns a local
Braille activity mark and elapsed label; a call waiting to run stays on a static
open mark. Completion lands briefly in its outcome colour before cooling into
the transcript, and newly arrived evidence follows the same restrained motion.
The animation registry is renderer-local, pauses away from the live tail, and
is bypassed by reduced-motion mode. Settled execution duration is durable.
`run_command` updates that same block with the
bounded, redacted capture while the process runs. Collapsed command output
shows the newest rows because verdicts land at the end. Every collapsed file
diff uses the same 15-changed-row budget, keeps both the beginning and end, and
inserts one omission summary when necessary. Diffs retain old/new line numbers
and intra-line emphasis. `Ctrl+O` toggles the latest detail even while an
approval is open, so reviewing, exporting, or expanding never depends on
discarded rows.

Reasoning follows the same retention rule. Its semantic block stores the full
stream, while the unframed default renderer reflows it at the current terminal
width and shows only the newest three muted, italicized visual rows. It has no
label or inline action hint; the visual treatment already identifies it, and
`/help` documents `Ctrl+O`. The final three-row preview remains visible.
`Ctrl+O` exposes the complete block; resize reflows the source again before
selecting the visible tail.

The viewport is bottom-relative. At offset zero it follows output. After the
user scrolls up, growth below the viewport increases the offset by the same
number of rows, preserving what is being read; the footer reports unseen
blocks. Tiny terminals receive a fixed recovery frame instead of fabricated
dimensions or overflowing chrome.

Jecode exposes one dark Slate identity through semantic colour tokens shared by
every production component and the TUI Lab. Structural Slate, technical cyan,
foreground, secondary, dim, and outcome roles are intentionally distinct;
components never embed literal colours. `NO_COLOR` disables colour while
retaining structural selection and state marks. Activity state marks remain
structural without colour. Reduced-motion mode renders resting tool states
directly and also makes the input cursor steady. Assistant answers animate only
through real provider streaming; the renderer never simulates typewriter output.

## Validation boundaries

Pure translation, layout, width, activity, scroll, permission, and transcript
logic is unit tested. Integration tests cover the packaged command, bootstrap
routing, batch conversations, durable session recovery, context projection and
compaction, TUI screen ownership and restoration, stream assembly, HTTP retry
and cancellation, provider request bodies, tool-loop semantics,
symlink/junction confinement, atomic preview checks, shell process-tree
termination, credential precedence, and bounded search.

Canonical checks:

```powershell
npm run check
```

The check enforces a source-only Git tree, type safety, line/branch/function
coverage thresholds, zero runtime dependencies, a freshly compiled runtime,
and a bounded package containing only the executable, plain JavaScript runtime,
license, manifest, and README. It then installs that package into an isolated
global prefix and runs its version command.

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
