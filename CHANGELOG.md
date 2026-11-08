# Release notes

## Unreleased

- Long conversations use deterministic native records of earlier work without a
  model request, with references to the original conversation. Model summaries
  remain for when records and previews cannot make room.
- Claude prompt caches default to five minutes; one-hour retention remains
  available for breaks.
- Unix terminal errors identify a failed control read, resize or interrupt,
  and distinguish malformed frames and closed channels from system errors.
- Commands remain visible and controllable during their initial wait, and long
  commands return control without stopping the process.
- `/process` replaces `/terminal`, with running and recent commands, saved output
  and a control to move a waiting call into the background without restarting it.
- Finished processes report their outcome to the model and conversation. The TUI
  can continue automatically; plain chat continues after the next entered line.

## 0.3.5

Changes since 0.2.5.

### Highlights

- **Workspace runs commands on Linux and macOS.** In 0.2.5 the default permission
  level refused `bash` and `exec` outside Windows. They now run under
  [operating-system confinement](docs/usage/PERMISSIONS.md#command-confinement):
  Landlock on Linux and Seatbelt on macOS. Hosts without that support still need
  the Full level.
- **More systems.** The npm package adds macOS on Intel and lowers the Linux
  floor from glibc 2.39 to 2.28. Release checks also enforce floors of macOS 11
  and Windows 10 1809, though not every floor has run Jecode yet; see the
  [evidence for each one](docs/development/PLATFORMS.md#supported-versions).
- **Lower token use.** A [five-turn benchmark](docs/development/AGENT_BENCHMARK.md)
  recorded fewer weighted tokens than Codex and Claude Code on one project, with
  all hidden acceptance tests passing. With GPT, Codex finished sooner.
- **A new terminal design.** Auto, Dark and Light themes keep your terminal's own
  background, and commands and settings open in place of the composer without
  touching your draft.
- **Language servers.** Definitions, references and diagnostics through servers
  you [enable yourself](docs/usage/LSP.md).

### Model efficiency

- Claude models keep the prompt cache for one hour by default, so a break no
  longer rewrites the conversation into the cache.
- Collaborating agents share the main conversation's prompt cache.
- `read` and `edit` accept several files in one call, and `steps` lets the model
  prepare several tools in one request while keeping each step's permissions,
  checkpoints and card.

### Connections

- Native HTTP adapters reuse connections between requests on Windows, Linux and
  macOS. OpenAI conversations use WebSocket with HTTP fallback.
  [Windows measurements](docs/development/PERFORMANCE.md#provider-transports)
  record lower first-update latency after the change.

### Terminal interface

- `/settings` → **Theme** chooses **Auto**, **Dark** or **Light**.
- Ctrl+G opens the commands in a sheet with its own filter while your draft waits
  above it; Esc goes back one level.
- Enter accepts the selected slash command before queueing it during a turn,
  so a prefix such as `/ne` waits as `/new` instead of an unknown command.
- Tool badges, status text, the startup screen and the browser sign-in pages share
  the new design.
- Long approval requests scroll with PgUp and PgDn and keep their choices on screen.
- A question or checkpoint finishing in the background no longer takes the keyboard
  from the panel you are using.

### Tools and commands

- The `bash` tool adds here-documents, here-strings, path patterns and the `grep`,
  `head` and `tail` built-ins; see [Command language](docs/usage/SHELL.md).
- `write` creates a new file's missing parent directories.
- In Workspace, commands can read Node.js runtimes installed through Volta, nvm,
  nvm-windows or fnm, and macOS locates the SDK first so Rust builds keep their
  platform headers.

### Upgrade notes

- **Prompt cache cost.** One-hour cache writes cost more: twice the input price
  instead of 1.25 times, at Anthropic's list prices. If you rarely pause, choose
  **5 minutes** in `/settings` → **Context** → **Prompt cache**.
- **Permissions on Linux and macOS.** If you chose Full only because Workspace
  refused commands, `/permissions` can take you back. Tools outside the permitted
  locations, such as those installed with pipx or uv, still need Full.
