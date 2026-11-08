# Release notes

## Unreleased

## 0.4.5

Changes since 0.3.5.

### Long work and context

- Long conversations use deterministic native records of earlier work without a
  model request, with references to the original conversation. Model summaries
  remain for when records and previews cannot make room.
- Claude prompt caches default to five minutes; one-hour retention remains
  available for breaks. Configurations saved by 0.3.5 already record its
  one-hour default and keep it: choose **5 minutes** in `/settings` →
  **Context** → **Prompt cache** to adopt the new default. One-hour writes cost
  more at Anthropic's API list prices; see
  [Configuration](docs/usage/CONFIGURATION.md#prompt-cache).

### Tools and browser

- Ambiguous edits return bounded match excerpts to help find a unique match
  before retrying. Prepared `steps` calls no longer have a fixed
  entry-count limit; shared collaborators can inspect recorded edit hunks.
- The built-in `wc` handles line and byte counts for files or standard input.
- Browser `type` and `press` send keyboard events, including shortcuts, Enter and
  Tab. Browser calls report blocking JavaScript dialogs so they can be handled
  before continuing.
- Commands remain visible and controllable during their initial wait, and long
  commands return control without stopping the process.
- `/process` replaces `/terminal`, with running and recent commands, saved output
  and a control to move a waiting call into the background without restarting it.
- Stopping a Unix terminal waits for the native cleanup result and final output.
  Failed or unconfirmed cleanup remains an unknown outcome.
- Finished processes report their outcome to the model and conversation. The TUI
  can continue automatically; plain chat continues after the next entered line.

### Development

- On macOS, resizing a PTY after its target exits now checks the watcher's exit
  state and waits for confirmed cleanup before returning the child's status.
  Resize failures while the target is running still report an error. A closed
  PTY also stops being polled until final cleanup, avoiding a reproduced busy
  loop while preserving output and exit codes.
- Unix terminal errors identify a failed control read, resize or interrupt,
  and distinguish malformed frames and closed channels from system errors.
- GitHub Actions dependencies were updated. Offline context checks screen saved
  checkpoints and sustained record growth without provider calls; they do not
  establish a ten-hour live result. See
  [context checks](docs/development/CONTEXT_CHECKS.md) and the
  [benchmark conditions](docs/development/AGENT_BENCHMARK.md).

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
