# Changelog

This file records notable changes in stable Jecode releases. Prereleases are
omitted. Extended notes for 0.1.1 and later are available on [GitHub Releases];
install artifacts and provenance are published with the [npm package].

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
