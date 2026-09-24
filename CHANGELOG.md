# Changelog

## Unreleased

- Incremental canonical session logs, bounded recovery and explicit verified v1
  import for long-running conversations.
- Ordered reference slices cover a single call and receipt larger than a bounded
  compaction request without replaying tools.

## 0.1.0-alpha.2 - 2026-09-22

- Minimal startup with directory, active model and effort in a single footer row.
- Command-only slash menu and selection controls inside the expandable composer.
  Help and context reports go to scrollback and leave the input clear.
- Use Ctrl+Q to save and exit; removed the redundant `/quit` command.
- Explicitly release local state locks so Linux session resume does not wait for
  a descriptor temporarily inherited by a concurrently starting process.

## 0.1.0-alpha.1 - 2026-09-22

- Inline terminal conversation with streamed OpenAI Account responses.
- Owned TLS, HTTP, JSON, terminal input and rendering.
- Workspace listing, reading and search; approved file creation, editing and commands.
- Local access outside the starting directory, absolute effect previews and saved
  access profiles; earlier sessions retain their original workspace boundary.
- Private JSON credentials, automatic access refresh and local logout.
- User settings, saved sessions, readable session cards and numbered resume
  selection without tool replay.
- Queued guidance, local command completion, context measurements and compaction.
- Start with `jecode` in the current directory; use `jecode resume` to reopen work.
- Filterable in-app command, resume, model and settings menus with keyboard selection.
- Ordered model changes retain the conversation and survive resume.
- Windows x64 native npm package and Linux x64 release archive.
- Offline behavioral tests and dependency, formatting and lint checks.

This is an alpha. Supported boundaries and limitations are documented alongside
the features; a listed feature is not a claim of independent security audit or
general performance superiority.
