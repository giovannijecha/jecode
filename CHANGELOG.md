# Changelog

This file records notable changes in stable Jecode releases. Prereleases are
omitted. Extended notes for 0.1.1 and later are available on [GitHub Releases];
install artifacts and provenance are published with the [npm package].

## Unreleased

### Changed

- Kept provider request identity and tool-result excerpts stable across growing
  turns so retries and ordinary continuation preserve cacheable prefixes.
- Made context estimation and compaction planning incremental, kept session
  checkpoints constant-time, and bounded catalogue reads and resume loads.
- Replaced external search acceleration with Jecode's owned bounded scanner,
  carrying verified file generations from discovery through descriptor reads.
- Updated development type checking to TypeScript 7 and current Node.js types;
  the shipped runtime still has zero third-party dependencies.
- Kept the selected provider route visible in the footer and distinguished
  separately billed OpenAI API models from ChatGPT account models in provider
  and model menus.
- Normalized provider failures across adapters and attached per-request client
  identifiers to OpenAI API traffic for support correlation.

### Fixed

- Hardened automatic and overflow compaction as distinct recovery paths,
  including cancellation, circuit-breaker reset, and successful retry coverage.
- Made session and cross-process locks generation-safe, required exact
  store-issued ownership for checkpoints, and bounded every durable read.
- Anchored session, settings, credential, and OAuth storage to canonical direct
  directories; mutations now preserve malformed or unknown state instead of
  silently rewriting it.
- Rejected workspace files replaced between discovery and search, normalized
  legacy lease races, and preserved the primary failure during cleanup.
- Retried only explicit transient generation rate limits, while treating
  billing and quota exhaustion as immediate actionable failures.
- Replaced a runner-sensitive context-estimation wall-clock assertion with its
  deterministic responsiveness and test-deadline guarantees.

## [0.8.4] - 2026-09-04

### Changed

- Organized `/providers` into Account and API access, with ChatGPT under
  Account and Anthropic, OpenAI API, and Ollama under API.
- Synchronized OpenAI and ChatGPT activity with response, reasoning, text, and
  tool-call lifecycle events so the footer reflects the current work phase.

### Fixed

- Bounded OpenAI and ChatGPT stream inactivity by complete SSE events so
  heartbeat traffic and incomplete framing cannot hide a stalled response.

## [0.8.3] - 2026-09-03

### Changed

- Kept context estimation and compaction planning responsive for multi-megabyte
  requests, with cancellation-aware work and bounded event-loop stalls.
- Made durable session checkpoints incremental while retaining complete
  validation during resume and crash recovery.
- Simplified the public README, moved detailed usage into a dedicated guide,
  and gave documentation and brand assets canonical repository locations.

### Fixed

- Coordinated process shutdown across the TUI, pending resume selection,
  provider requests, tools, and batch input before terminal restoration.
- Rejected malformed provider tool arguments before preview or execution.
- Bounded interactive and batch prompt input without splitting Unicode
  characters at decoder chunk boundaries.
- Bounded settings, API-key, and OAuth stores before parsing and writing, and
  made credential redaction fail closed beyond its supported limits.
- Requested streaming usage from compatible Ollama endpoints so context
  pressure can use provider-reported input when available.
- Revalidated session directories during incremental checkpoint writes so a
  replaced junction or symlink cannot redirect durable data.

## [0.8.2] - 2026-09-03

### Changed

- Reused validated request token estimates until compaction changes the
  provider-facing projection, avoiding duplicate work on large prompts.
- Cached Ollama's theoretical model capacity within the existing metadata
  window while continuing to prefer a smaller active runtime allocation.
- Loaded durable session nodes in bounded parallel batches while preserving
  their canonical order and complete validation.
- Restored a brighter Slate palette and a subtle full-width surface for user
  turns, while keeping reasoning unframed and execution rails exclusive to
  adjacent tool calls.

### Fixed

- Preserved every completed result in a concurrent tool batch when another
  call is interrupted or result rendering fails, and settled active calls
  before checkpointing.
- Hardened ChatGPT device authorization polling with protocol-aware backoff,
  immediate terminal-error handling, and one bounded deadline across network
  requests, waits, and cancellation.

## [0.8.1] - 2026-09-03

### Fixed

- Aligned assistant prose, reasoning rails, and tool state marks at the terminal
  left edge, with continuous visual rhythm between adjacent reasoning and tool
  evidence.
- Kept settled tool outcomes visibly green or red and preserved technical color
  and weight for inline code nested inside Markdown emphasis, including the
  accessible `NO_COLOR` fallback.

## [0.8.0] - 2026-09-03

### Added

- Added cooperative mid-turn steering from the composer. Guidance enters the
  active turn at the next safe provider or tool boundary, while interruption
  and checkpoint failure restore anything the model did not receive.

### Changed

- Reworked the terminal transcript around one adaptive semantic gutter, a
  calibrated Slate palette, continuous tool evidence, quieter reasoning, and
  bounded renderer-local motion that preserves reduced-motion, `NO_COLOR`,
  narrow-terminal, resume, and export behavior.
- Removed the default model-request ceiling from ordinary turns. `--max-steps`
  and `JECODE_MAX_STEPS` remain explicit process-only budgets for deterministic
  automation.
- Persisted settled tool durations through export and resume with session schema
  4, while retaining strict read compatibility with schemas 1 through 3.
- Defined the compatibility, release-candidate, and stabilization contract that
  Jecode will carry into 1.0.

### Fixed

- Synchronized footer activity with provider preparation, local tool preview,
  approval, execution, and response phases instead of inferring work from the
  previous transcript block.
- Refreshed rotated ChatGPT credentials that are already near expiry before
  retrying a rejected request, without weakening shared refresh or persistence
  locking.
- Rejected incomplete or truncated tool calls from Anthropic, OpenAI, ChatGPT,
  and Ollama before execution while preserving safe partial assistant text.
- Discarded suppressed tool motion after settlement so reduced-motion state
  cannot replay later or remain retained across transcript replacement.

## [0.7.4] - 2026-09-02

### Fixed

- Made fragmented SSE event parsing linear so large reasoning and tool payloads
  cannot repeatedly rescan an incomplete prefix and stall provider streaming.
- Kept localized edits in large files as localized diffs instead of displaying
  and persisting the complete file as one coarse replacement.
- Preserved small provider context capacities after adapter headroom instead of
  replacing them with the much larger unknown-model fallback.
- Replaced stale context pressure with the sent request estimate when a
  provider omits usage, without inventing vendor-reported token totals.
- Prevented an interrupted `write_file` or `edit_file` from crossing its atomic
  rename boundary, while still cleaning the temporary sibling safely.
- Made long unbroken words wrap in linear time without losing grapheme or cell
  boundaries in Markdown and plain terminal text.
- Kept bounded previews, provider and OAuth error details, and colored diff
  emphasis on complete grapheme boundaries, including historical emphasis.
- Stopped stable documentation from advertising an inactive npm `next` channel
  and added a release gate against future drift.

## [0.7.3] - 2026-09-02

### Fixed

- Clamped the complete provider request, including prompt, messages, tool
  schemas, reasoning, and output, to the selected model's usable capacity.
- Revalidated approved file contents immediately before atomic replacement so
  concurrent editor or process updates are preserved instead of overwritten.
- Made durable conversation restore and checkpoint validation scale linearly
  across long branched sessions.
- Preserved complete Unicode grapheme clusters when file, search, and command
  output reaches its configured boundary.
- Kept large-transcript resize responsive through incremental reflow and a
  bounded two-width layout cache.
- Scaled aggregate SSE protection with the effective model output budget while
  retaining per-event, absolute, timeout, and cancellation limits.
- Replaced the remaining pending-tool animation with one stable state mark and
  a one-second elapsed-time refresh.
- Parallelized bounded workspace text-search validation and portable reads
  without changing canonical order, cancellation, or result limits.

## [0.7.2] - 2026-09-02

### Fixed

- Preserved partial transcript evidence and explicit failed or interrupted
  outcomes consistently across export, timeline, resume, and later turns.
- Enforced the same per-field and UTF-8 file-size boundaries before session
  commits and writes that the loader applies after a restart, while retaining
  compatibility with schema 1 and 2 sessions.
- Ordered the resume catalogue by durable update time across its complete
  bounded input set, so an older session updated most recently stays visible.
- Aborted and awaited active provider or tool work before closing persistence
  after a fatal TUI failure, and preserved failure outcomes through `/compact`.

## [0.7.1] - 2026-09-02

### Fixed

- Prevented a longer credential that shares another secret's prefix from
  exposing its suffix when shell output crosses stream chunks.
- Normalized malformed provider token counters before usage accounting or
  durable session persistence can consume them.
- Restored terminal, input, resize, timers, and persistence after initial or
  scheduled TUI rendering failures.
- Rejected missing launch-option values and stray positional arguments instead
  of silently accepting a misconfigured process.
- Neutralized every Unicode bidirectional control before terminal rendering
  and made searchable-picker deletion operate on complete grapheme clusters.

## [0.7.0] - 2026-09-02

### Added

- Added a searchable `/timeline` conversation tree that exposes completed
  turns and lets users select an earlier branch point without creating or
  persisting an empty branch.
- Added append-only branching within one durable session. The next real user
  turn from a selected point persists the alternate path, while historical
  tool records remain inert and resume returns to the latest committed branch.
- Added `/compact` to request the existing model-aware compaction policy
  immediately in interactive and batch modes. Only the active leaf's context
  anchor changes; canonical history, the visible transcript, and exports stay
  complete.

### Fixed

- Prevented manual compaction from rewriting a temporarily selected historical
  branch point, and kept cancelled, unchanged, or undersized operations silent.
- Removed the legacy leading marker from informational command feedback and
  corrected the context-compaction field's corrupted keyboard separator.

## [0.6.0] - 2026-09-01

### Added

- Added automatic model-aware context compaction whose budget follows the
  selected model's usable window instead of a fixed token threshold. Live
  provider metadata, Ollama's active allocation, conservative family limits,
  and a safe fallback keep the policy useful across small and million-token
  contexts.
- Added one saved compaction trigger from 50% to 95%, defaulting to 85%,
  available through `/settings`, `--compaction-percent`, and
  `JECODE_COMPACTION_PERCENT`; provider safety limits still take precedence.
- Persisted branch-local compaction anchors with durable sessions so resumed
  conversations reuse the compact provider projection while the canonical
  conversation tree and exported transcript remain complete.

### Fixed

- Recovered once from definite provider context-overflow responses by
  compacting and retrying without duplicating canonical conversation history.
- Kept Ollama's theoretical model capacity from hiding a later, smaller active
  runtime allocation when deciding the safe context budget.
- Kept every settings row visible at compact terminal heights and shortened the
  provider-management hint before it needs truncation.

## [0.5.0] - 2026-09-01

### Added

- Added workspace-scoped durable TUI conversations with `jecode resume`, a
  searchable session picker, `resume --latest`, and an explicit `--ephemeral`
  mode. Repeated resumes advance one stable logical session and never replay
  historical tools.
- Added a canonical conversation tree shared by provider history, settled TUI
  transcript state, crash-safe checkpoints, internal branches, and the future
  timeline surface.

### Fixed

- Made foreground activity expose a compact footer state and elapsed timer
  without another spinner, while preserving warning and error priority.
- Published a fresh durable session with its process lease already present,
  preventing another process from claiming it between publication and use;
  lease reads now also reject links, replacements, and oversized files.
- Kept failed resume identities from partially changing the current runtime
  selection, and normalized multiline session previews into one readable row.
- Exposed Ollama's supported positive reasoning levels (`low`, `medium`, and
  `high`) and now pass the selected value through `reasoning_effort` instead of
  reporting every Ollama model as model-controlled.
- Removed the redundant `thinking` / `thought` heading and inline action hint
  from reasoning blocks; the muted text remains expandable through `Ctrl+O`.
- Bounded collapsed file diffs to 15 changed rows with one omission summary,
  kept full inspection available during approvals, and removed the false blank
  deletion previously shown for newly created files.

## [0.4.0] - 2026-09-01

### Added

- Added one searchable model catalogue across every currently available
  provider, with concurrent loading and partial-failure isolation.

### Changed

- Made model selection an atomic provider-and-model choice while retaining the
  existing saved-settings and CLI compatibility contract.
- Consolidated API keys, ChatGPT OAuth, and Ollama connections under
  `/providers`; removed the redundant `/credentials` command and made
  `/settings` model-first.
- Simplified direct control menus by removing redundant headings and guidance;
  `/permissions` now changes each tool policy inline with Left/Right while
  Enter opens only that tool's remembered approvals.
- Expanded composer editing with word navigation and deletion through
  Ctrl+Left/Right, Ctrl+Backspace, and Ctrl+Delete across common terminal
  encodings, including Windows Terminal and VS Code.
- Raised transcript contrast and separated structural Steel, technical cyan,
  readable secondary text, and deliberately dim metadata into distinct roles.
- Replaced full-row menu selection bands with a bold Steel focus on the active
  command or value while retaining the arrow fallback under `NO_COLOR`.

### Fixed

- Preserved provider identity and current setting values in minimum-width
  menus instead of hiding them with optional descriptive hints.
- Kept cancellation during model-catalogue, effort discovery, and nested
  provider management on the interruption path without reopening menus or
  leaving redundant footer warnings.

## [0.3.2] - 2026-09-01

### Fixed

- Kept compact edit previews focused on changed lines while preserving full
  context in the expanded view.
- Increased the contrast of selected rows throughout TUI menus.

## [0.3.1] - 2026-09-01

### Fixed

- Kept long-running OAuth refreshes serialized even after a store lock passes
  its stale-age threshold, and made abandoned-lock recovery safe with multiple
  waiting processes.
- Bounded ripgrep accelerator events by the requested global search limit and
  fell back to the portable scanner when ripgrep's per-file limit would exceed
  it.

## [0.3.0] - 2026-09-01

### Added

- Added bounded parallel execution for consecutive read-only tool calls while
  preserving original result order and keeping writes, commands, approvals,
  and unknown tools as ordered barriers.
- Added optional native `rg` acceleration for large literal searches, with the
  dependency-free scanner retained as the portable fallback.

### Changed

- Refreshed the dark Steel palette with clearer structural blues and distinct
  success, warning, and failure colors while preserving the established TUI
  layout and interaction model.

### Fixed

- Resolved native helpers to canonical absolute executables outside the active
  workspace, preventing repositories from shadowing `rg` or browser and
  process-control launchers.
- Force-terminated command descendants that survive graceful timeout or
  interruption after their shell leader exits.
- Serialized settings, API-key, and OAuth-account mutations through bounded
  cross-process locks that reread each store before writing.
- Treated an omitted or empty `list_dir` path as the workspace root and kept
  rapid transcript exports distinct with millisecond timestamps.

## [0.2.4] - 2026-08-31

### Fixed

- Kept the model-facing system prompt product-neutral so assistant identity can
  come only from explicit workspace content.

## [0.2.3] - 2026-08-31

### Fixed

- Refreshed shell redaction snapshots so API and OAuth credentials rotated by
  another Jecode process remain hidden.
- Rejected blank or duplicate tool-call identifiers before executing tools or
  changing conversation history.
- Limited `read_file` to cancellable regular-file reads, avoiding hangs on
  FIFOs and other special files.
- Preserved Ollama reasoning across compatible tool-call continuations.

## [0.2.2] - 2026-08-31

### Fixed

- Kept split and unbound terminal escape sequences out of the composer, used a
  single escape timer, and made an interrupted bracketed paste recoverable.
- Bounded shell timeout scheduling and output-pipe draining when detached
  descendants outlive the command that launched them.
- Replaced model-authored glob regular expressions with bounded,
  predictable-complexity wildcard matching.
- Avoided over-redacting ordinary text that matches short heuristic
  environment secrets while retaining explicit API and OAuth redaction.
- Repaired tool-result history after unexpected approval or display failures.
- Restored the terminal before crash diagnostics, preserved conventional signal
  exit codes, and corrected cell widths for default emoji and regional flags.

## [0.2.1] - 2026-08-31

### Changed

- Limited provider model and effort menus to capabilities that each transport
  can satisfy, including the authenticated ChatGPT model catalogue.
- Bounded live reasoning rendering while retaining complete thought content
  for expansion and transcript export.

### Fixed

- Kept provider history and tool rails consistent when a tool batch is
  interrupted.
- Preserved replacement tokens such as `$$` and `$&` literally in `edit_file`.
- Sent immediate EOF to shell commands and reported non-zero exits and timeouts
  as tool errors.
- Completed OAuth refresh-token rotation independently from caller
  cancellation.
- Rejected Anthropic, OpenAI, and Ollama streams that end without their required
  terminal event.
- Preserved SSH agent access for approved commands without forwarding
  secret-bearing environment variables.

## [0.2.0] - 2026-08-31

### Added

- Added an experimental `openai-codex` provider that signs in with ChatGPT via
  browser PKCE or device code, refreshes rotated tokens safely, and remains
  separate from the existing OpenAI API-key provider.
- Added connected-account management to `/settings` and `/credentials`, with
  owner-only OAuth storage under `~/.jecode/accounts.json`.

### Changed

- Generalized provider authentication so API keys and OAuth accounts retain
  distinct persistence, feedback, and setup flows.
- Preserved ChatGPT Codex tool calls when its final streamed response carries
  an empty output envelope after complete output-item events.
- Finished OpenAI streams at their terminal event, rejected malformed SSE, and
  surfaced empty provider replies instead of ending a turn silently.
- Hid the unsupported max-output-token setting while OpenAI Codex is active.
- Branded the browser callback, made it close the TUI wait state reliably, and
  kept authorization details out of the completed browser URL.
- Decoupled ChatGPT model-catalogue compatibility from Jecode's own version so
  eligible Codex models remain available.

### Fixed

- Rejected invalid OAuth callback state without cancelling the valid sign-in
  still in progress.

## [0.1.9] - 2026-08-31

### Added

- Added a session-only `/permissions` control plane for every built-in tool,
  with allow, ask, and deny policies plus granular remembered approvals.
- Added a compact `/help` reference inside the composer dock.

### Changed

- Kept slash commands and operational feedback out of conversation transcripts
  and Markdown exports, while leaving token accounting internal.
- Simplified footer feedback, menu navigation, and row alignment around one
  consistent composer interaction model.
- Removed the redundant `/setup` and public `/usage` commands; `/settings` now
  remains the single persistent configuration surface.

### Fixed

- Kept the Jecode wordmark legible on npm and other README renderers with dark
  backgrounds.

## [0.1.8] - 2026-08-31

### Added

- Added verified Node.js 22.18 LTS support alongside Node.js 24, with CI coverage
  across Windows, Ubuntu, and macOS for both runtimes.
- Added focused regression coverage for credential commands, TUI input and
  overlays, and the OpenAI wire protocol.

### Changed

- Clarified migration from the legacy unscoped package and documented a clean,
  Linux-native WSL installation that cannot collide with Windows shims.

## [0.1.7] - 2026-08-31

### Fixed

- Terminal batch failures now write diagnostics to stderr and exit non-zero.
- Incomplete streamed answers are discarded when a provider fails instead of
  being flushed as successful output.

## [0.1.6] - 2026-08-31

### Changed

- Redesigned the terminal workflow around one compact execution rail,
  composer-native menus, unframed reasoning, and a one-line status footer.
- Added bounded, credential-redacted live output for `run_command` while
  retaining complete results for expansion and transcript export.
- Replaced the implicit package build with explicit release commands and a
  source-tree guard, without adding install lifecycle scripts.

## [0.1.5] - 2026-08-30

### Added

- Published the first stable `@giovannijecha/jecode` package on npm.
- Added `latest` and opt-in `next` channels with trusted OIDC publishing, npm
  provenance, and complete release gates on Windows, Ubuntu, and macOS.
- Removed generated JavaScript from Git while continuing to include a compiled,
  dependency-free runtime in the npm package.

## [0.1.4] - 2026-08-30

### Added

- Added Ollama Cloud, local daemon, and custom endpoint configuration to
  `/settings`, with persistent defaults under `~/.jecode`.

### Fixed

- Restored Ollama Cloud automatically for saved API keys while preventing Cloud
  credentials from reaching local endpoints.
- Replaced raw Ollama connection failures with actionable settings guidance.

## [0.1.3] - 2026-08-30

### Security

- Isolated credential-like environment values from shell commands and redacted
  known secrets before output reached the model, transcript, or export.
- Bounded provider requests and responses, rejected redirects, separated
  handshake and idle deadlines, and prevented generation request replays.
- Hardened workspace path confinement, atomic writes, preview races, mutation
  budgets, and terminal-control neutralization.

### Changed

- Made the local Ollama daemon the default and required HTTPS for remote hosts.

## [0.1.2] - 2026-08-30

### Fixed

- Made the committed JavaScript runtime deterministic across Windows checkouts
  by enforcing LF line endings.

## [0.1.1] - 2026-08-30

### Fixed

- Shipped a script-free JavaScript runtime for direct GitHub installations.
- Added clear unsupported-Node diagnostics and cross-platform installed-package
  validation.
- Documented installation, updates, uninstallation, and WSL path requirements.

## [0.1.0] - 2026-08-29

### Added

- Released the initial single-controller coding loop with streaming Anthropic,
  OpenAI, and Ollama providers, visible tools, approvals, and the full-screen
  terminal interface.
- Published the repository documentation, support policies, brand assets, and
  initial CI and package gates.

[GitHub Releases]: https://github.com/giovannijecha/jecode/releases
[npm package]: https://www.npmjs.com/package/@giovannijecha/jecode
[0.8.4]: https://github.com/giovannijecha/jecode/releases/tag/v0.8.4
[0.8.3]: https://github.com/giovannijecha/jecode/releases/tag/v0.8.3
[0.8.2]: https://github.com/giovannijecha/jecode/releases/tag/v0.8.2
[0.8.1]: https://github.com/giovannijecha/jecode/releases/tag/v0.8.1
[0.8.0]: https://github.com/giovannijecha/jecode/releases/tag/v0.8.0
[0.7.4]: https://github.com/giovannijecha/jecode/releases/tag/v0.7.4
[0.7.3]: https://github.com/giovannijecha/jecode/releases/tag/v0.7.3
[0.7.2]: https://github.com/giovannijecha/jecode/releases/tag/v0.7.2
[0.7.1]: https://github.com/giovannijecha/jecode/releases/tag/v0.7.1
[0.7.0]: https://github.com/giovannijecha/jecode/releases/tag/v0.7.0
[0.6.0]: https://github.com/giovannijecha/jecode/releases/tag/v0.6.0
[0.5.0]: https://github.com/giovannijecha/jecode/releases/tag/v0.5.0
[0.4.0]: https://github.com/giovannijecha/jecode/releases/tag/v0.4.0
[0.3.2]: https://github.com/giovannijecha/jecode/releases/tag/v0.3.2
[0.3.1]: https://github.com/giovannijecha/jecode/releases/tag/v0.3.1
[0.3.0]: https://github.com/giovannijecha/jecode/releases/tag/v0.3.0
[0.2.4]: https://github.com/giovannijecha/jecode/releases/tag/v0.2.4
[0.2.3]: https://github.com/giovannijecha/jecode/releases/tag/v0.2.3
[0.2.2]: https://github.com/giovannijecha/jecode/releases/tag/v0.2.2
[0.2.1]: https://github.com/giovannijecha/jecode/releases/tag/v0.2.1
[0.2.0]: https://github.com/giovannijecha/jecode/releases/tag/v0.2.0
[0.1.9]: https://github.com/giovannijecha/jecode/releases/tag/v0.1.9
[0.1.8]: https://github.com/giovannijecha/jecode/releases/tag/v0.1.8
[0.1.7]: https://github.com/giovannijecha/jecode/releases/tag/v0.1.7
[0.1.6]: https://github.com/giovannijecha/jecode/releases/tag/v0.1.6
[0.1.5]: https://github.com/giovannijecha/jecode/releases/tag/v0.1.5
[0.1.4]: https://github.com/giovannijecha/jecode/releases/tag/v0.1.4
[0.1.3]: https://github.com/giovannijecha/jecode/releases/tag/v0.1.3
[0.1.2]: https://github.com/giovannijecha/jecode/releases/tag/v0.1.2
[0.1.1]: https://github.com/giovannijecha/jecode/releases/tag/v0.1.1
[0.1.0]: https://github.com/giovannijecha/jecode/tree/v0.1.0
