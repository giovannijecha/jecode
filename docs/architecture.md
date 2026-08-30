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
npm pack      → clean build → dist/ inside the release tarball
```

There are no subagents, workers, delegated tasks, or concurrent controller
loops. Complex work advances through more iterations of the same loop.

## One turn

`runTurn` owns the protocol:

1. Send normalized history, tool declarations, and request settings to the
   selected provider.
2. Stream display events while the provider assembles the authoritative
   assistant message.
3. Append that message to history and record normalized usage.
4. Return when it contains no tool calls.
5. Otherwise preview each call, request approval when required, run it, and
   collect every result from the step into one user message.
6. Repeat up to `maxSteps`.

A tool failure becomes an error result the model can act on. Cancellation is
the exception: it propagates through provider HTTP, retry waits, filesystem
searches, and process trees so the foreground operation can end promptly.

## Foreground activity

The interactive shell permits one foreground activity: a slash command or a
model turn. One `AbortController` owns its cancellation. This
prevents a network-backed `/models` request from overlapping a turn and gives
`Esc`, `Ctrl+C`, and `Ctrl+D` one consistent target.

There is no startup banner, permanent preamble, or automatic menu. Every launch
opens on an empty transcript and composer; readiness remains visible in the
footer, and `/settings` opens only when the user invokes it. Its nested
provider, model, and credential flows use the same dock interaction as every
other selector. Cancelling a new provider leaves the previous provider and
model unchanged.

## Provider boundary

The controller uses the normalized types in `src/types.ts`: messages contain
text, tool calls, tool results, optional opaque provider data, and optional
usage. Providers translate this vocabulary to three different APIs:

| Provider | API | Deployment |
|---|---|---|
| Anthropic | Messages | Cloud |
| OpenAI | Responses | Cloud |
| Ollama | OpenAI-compatible Chat Completions | Local loopback or hosted |

Each provider has three responsibilities:

- `*-wire.ts` translates complete requests and responses.
- `*-stream.ts` assembles authoritative responses from events.
- `*.ts` applies authentication, model-specific fields, and HTTP transport.

Streamed text is display-only. The complete message returned by `send` is the
only assistant message appended to history. Anthropic thinking signatures and
OpenAI reasoning items are retained as provider-tagged opaque data so a later
request can echo them without corrupting or inventing fields. Cross-provider
history falls back to normalized content.

OpenAI Responses requests use `store: false` and request encrypted reasoning
content for stateless continuation. Refusals, incomplete responses, nested
failures, and usage are normalized rather than disappearing at the stream
boundary.

The shared HTTP client retries transient network errors, rate limits, and 5xx
responses only for idempotent catalogue GETs. Generation POSTs are never
replayed after an ambiguous network or server failure. A request has 60 seconds
to receive response headers, and an open JSON or SSE body can remain idle for
at most 120 seconds. These internal deadlines also cover batch mode, where no
interactive cancellation signal exists. The client handles redirects manually
and rejects every 3xx response without retrying or forwarding headers to
another endpoint. Retry state is surfaced in the TUI. Each SSE event, the
aggregate stream, reconstructed tool arguments, model catalogue, and per-step
tool-call batch have explicit limits; overflow cancels or rejects the response
before unbounded work reaches the controller.

Ollama initializes its endpoint from `--ollama-host`, `OLLAMA_HOST`, saved
settings, then key-aware inference. A configured API key selects
`https://ollama.com`; without one, Jecode targets the local daemon at
`http://127.0.0.1:11434`. The Ollama connection row in `/settings` can replace
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
the temporary pathname must still identify the file Jecode opened. A previewed
write or edit also compares the current file with the approved version, so an
intervening content change is rejected instead of overwritten.

Whole-file mutations accept regular files only and enforce independent byte,
character, and line budgets. Existing content is read through a bounded file
handle, and `replace_all` checks its projected size before allocating the
result. Files above that budget remain available through ranged `read_file`
calls, but `write_file` and `edit_file` will not replace them wholesale.

The built-in tools are:

| Tool | Approval | Boundary |
|---|---|---|
| `read_file` | No | Progressive UTF-8 read, bounded output, canonical path |
| `list_dir` | No | Bounded entries and output from one canonical directory |
| `find_files` | No | Bounded recursive glob; skips VCS, dependencies, symlinks |
| `search_text` | No | Bounded literal search; skips binary and files over 1 MB |
| `edit_file` | Yes | Bounded exact replacement, atomic write, preview check |
| `write_file` | Yes | Bounded whole-file replacement, atomic write, preview check |
| `run_command` | Yes | Workspace working directory, timeout, bounded output |

`run_command` is not a filesystem sandbox. A shell can address anything the
user account can address, which is why every exact command requires approval
unless the process was started with `--auto-approve`. On cancellation or
timeout, jecode terminates the process tree and escalates if it does not exit.
The child receives an explicit copy of the process environment with
credential-like names removed. Known environment, session, and saved
credential values are redacted from captured output before the controller can
send it to a provider or retain it in a transcript block.

The “allow this session” choice is deliberately narrow: one target file for
file changes, or one exact shell command. `/permissions` can revoke one grant
or all grants.

## Settings, credentials, and local data

Persistent user data lives under `~/.jecode`, outside every workspace.
`settings.json` contains only non-secret defaults: provider, one remembered
model per provider, the Ollama endpoint, effort, limits, and reduced motion. Runtime
precedence is CLI flags, environment variables, saved settings, then built-in
defaults. Root and auto-approval stay process-only.

Credentials resolve from environment, then in-memory session values, then the
saved `~/.jecode/credentials.json` file. The environment wins. The TUI never
prints a secret, and credential values never enter message history or
transcript blocks. Older config-directory credentials remain a read-only
fallback until the canonical file is first written.

Saving is explicit. The credential directory and file use owner-only modes on
POSIX; Windows relies on the user profile ACL. Replacement uses the same atomic
writer as workspace files. `/credentials` shows only `environment`, `session`,
`saved`, or `missing` and can remove a saved copy.

jecode writes no conversation state automatically. `/export` is an explicit,
argument-free operation that writes an automatically named Markdown transcript
in the directory from which Jecode was launched. It requires no picker or approval.
`/new` clears the in-memory history, usage, transcript, and session approvals.

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
The viewport is assembled from only the cached row ranges it intersects rather
than flattening the complete history on every frame. Cell measurement accounts
for graphemes, combining marks, emoji, and wide CJK glyphs. Markdown prose uses
a readable maximum measure while code and tables retain the available width.

Untrusted text is neutralized before both cell measurement and paint. C0, C1,
ESC/CSI/OSC, delete, bidirectional controls, and pasted control sequences
therefore remain visible data instead of terminal instructions. Renderer-owned
styling escapes are introduced only after this boundary.

The visual grammar follows a transcript rather than a dashboard or execution
diagram:

```text
user input       full-width neutral surface
reasoning        unframed muted label and three-row live tail
assistant        unframed Markdown
tool activity    full-width pending/success/failure surface
selection        shared arrow rows, with no selected card or state badge
composer         editor and contextual menu inside one pair of rules
footer           model/effort/workspace left, replaceable status right
```

Operational feedback is not conversation. Slash-command confirmations,
configuration guidance, foreground activity, and preflight blockers share the
replaceable right side of the one-line footer. Errors and warnings take
priority over activity; activity takes priority over informational feedback and
unseen output. Informational messages expire; warnings and errors remain
briefly or until the next key. They never become transcript blocks and
therefore never enter Markdown exports. Turn readiness is checked before the
editor, recall history, or model history is mutated, so a missing key or model
leaves the unsent prompt in place.

Conversation turns use one blank terminal row as their outer separator. User
and tool surfaces keep their own top and bottom padding inside that rhythm, so
alternating turns never visually collide with the prose before them.

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
selectors, and credential fields provide its inner rows; none draws its own
border. Autocomplete opens on `/`, keeps selection separate from the typed
prefix, and assigns each key one stable job: Up/Down select, Tab completes,
Enter runs, and Esc closes. The footer remains outside the shell, so it never
jumps when a menu opens.

Writable fields carry the same `→ ` active marker as selected menu rows. The
field renderer owns that prompt, including its terminal-cell width and cursor
offset, so numeric settings and masked credentials cannot drift apart.

Assistant Markdown follows the same restraint. Inline code is accent text,
not a background chip. Fenced code shows muted opening and closing fences with
a two-cell body indent; it has no full-width surface or decorative left rail.

Tool output and diffs stay complete in state. Diffs carry old/new line numbers
and intra-line emphasis. Rendering previews long output and `Ctrl+O` toggles
the latest detail, so exporting or expanding never depends on discarded rows.

Reasoning follows the same retention rule. Its semantic block stores the full
stream, while the unframed default renderer reflows it at the current terminal
width and shows only the newest three muted, italicized visual rows below its
label. The label reads `thinking` while the stream is live and `thought` once
it is sealed, without a separate live badge. The final three-row preview remains
visible. `Ctrl+O` exposes the complete block; resize reflows the source again
before selecting the visible tail.

The viewport is bottom-relative. At offset zero it follows output. After the
user scrolls up, growth below the viewport increases the offset by the same
number of rows, preserving what is being read; the footer reports unseen
blocks. Tiny terminals receive a fixed recovery frame instead of fabricated
dimensions or overflowing chrome.

Jecode exposes one dark Steel identity through semantic colour tokens shared by
every production component and the TUI Lab. `NO_COLOR` disables colour.
Reduced-motion mode replaces the animated spinner and blinking cursor with
stable marks.

## Validation boundaries

Pure translation, layout, width, activity, scroll, permission, and transcript
logic is unit tested. Integration tests cover the packaged command, bootstrap
routing, batch conversations, TUI screen ownership and restoration, stream
assembly, HTTP retry and cancellation, provider request bodies, tool-loop
semantics, symlink/junction confinement, atomic preview checks, shell
process-tree termination, credential precedence, and bounded search.

Canonical checks:

```powershell
npm run check
```

The check enforces type safety, line/branch/function coverage thresholds, zero
runtime dependencies, a freshly compiled runtime, and a bounded package
containing only the executable, plain JavaScript runtime, license, manifest,
and README. It then installs that package into an isolated global prefix and
runs its version command.

## Deliberate omissions

- No runtime dependency, SDK, curses layer, install-time compiler, or bundler.
  TypeScript remains the development source; the ignored release-only `dist/`
  tree is emitted by the existing development compiler while packing. Registry
  users receive that JavaScript runtime and execute no installation scripts.
- No automatic conversation persistence.
- No client-side history summarizer or second model loop. Usage and latest
  input context are visible; context compaction belongs at the provider seam
  when a supported server-side mechanism is selected.
