# Changelog

This file records notable changes in stable Jecode releases. Prereleases are
omitted. Extended notes for 0.1.1 and later are available on [GitHub Releases];
install artifacts and provenance are published with the [npm package].

## Unreleased

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
