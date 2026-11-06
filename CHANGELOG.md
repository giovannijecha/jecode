# Release notes

## 0.3.5 — Unreleased

This release brings the work merged since 0.2.5 into one native package for
Windows x64, Linux x64, and macOS on Apple silicon and Intel.

### Efficient model work

- Prompt caching keeps stable context reusable, with a one-hour Claude cache
  lifetime by default and shared instructions for collaborating agents.
- File tools accept batches, and `steps` lets the model prepare several tools
  in one request while keeping each step's permissions, checkpoints and card.
- A [five-turn benchmark](docs/development/AGENT_BENCHMARK.md) records lower
  weighted token use than Codex and Claude Code on one project, with all hidden
  acceptance tests passing. It also records the GPT time tradeoff.
- Native HTTP adapters reuse connections; OpenAI conversations use WebSocket
  with HTTP fallback. [Windows measurements](docs/development/PERFORMANCE.md#provider-transports)
  record lower first-update latency after the transport change.

### Native terminal interface

- Dark, light and automatic themes preserve the terminal's background.
- Commands and settings replace the composer while keeping the draft above
  them. Ctrl+G opens commands without editing the draft; Esc returns one level.
- Tool badges, status text, the startup screen and browser sign-in pages follow
  the same design. Async questions and checkpoint completion preserve which
  panel owns the keys.

### Tools and platforms

- Opt-in language-server connections provide definitions, references and
  diagnostics through the owned LSP adapter.
- MCP sign-in and connection handling include the OAuth and release-check
  fixes merged after 0.2.5.
- Linux and macOS share persistent HTTP transport and native terminal support.
  Release packaging enforces glibc 2.28, macOS 11 and Windows build 17763 floors;
  see [platform evidence and remaining manual checks](docs/development/PLATFORMS.md).
- macOS finds the SDK before confining build commands, so the compiler keeps
  access to its platform headers.

This version is pending review and publication.
