# Release notes

## Unreleased

Changes since 0.7.5.

- **Command line.** New `settings`, `providers`, `models`, `sessions`,
  `skills`, `mcp`, `lsp`, `doctor`, `init` and `completions` commands manage
  Jecode from a terminal through the same stores and checks as the TUI;
  inspection needs no model credentials. Every command has `--help`, and
  management commands print JSON with `--json`. See
  [Command line](docs/usage/CLI.md).
- **Run options.** `--provider`, `--model`, `--effort`, `--routing`, `--permissions` and
  `-C DIR` choose a conversation's route and folder without changing saved
  defaults. An explicit provider never falls back to another billing route.
  `jecode resume --last` and `jecode resume ID "PROMPT"` continue a
  conversation directly. `--json` prints a run as JSON Lines.
- **Prompt and piped data.** `git diff | jecode "Review this diff"` now sends
  both: the prompt, then the piped data. Oversized or non-UTF-8 input is an
  error rather than being ignored. Scripts that run Jecode with a stdin they
  do not mean to send, such as inside a `while read` loop, should redirect it
  from `/dev/null`.
- **Update and uninstall.** `jecode update` updates npm, Volta and standalone
  installations; standalone updates verify the published release and need no
  Node.js. `jecode uninstall` removes only the running installation and keeps
  your Jecode home. See [Installation](docs/usage/INSTALLATION.md#update-jecode).
- **Interface.** `/help` gains a **Shell** tab listing these commands, and
  Settings → Providers → API keys can remove a saved key with Ctrl+D.
- **Terminal title.** The interactive TUI sets the tab or window title to
  `Jecode` and restores the previous title on supported terminals. `--plain`,
  JSON and piped runs leave it unchanged. Unix sessions identified as Windows
  Terminal keep their title because that host lacks the title save/restore stack.
- **Parser.** `jecode --plain setup` and other inapplicable options are now
  usage errors (exit status 2) instead of being sent as a task.

## 0.7.5

Changes since 0.6.5.

- **Skill and file references.** Type `$` or `@` in the composer to choose a
  personal skill or browse project files. References appear as editable cards
  alongside attachments; Tab reopens a reference's selector. The model reads
  the referenced skill or file when needed, under the existing permission
  rules. See [Reference skills and files](docs/usage/INTERFACE.md#reference-skills-and-files).
- **Batch read approvals.** Reading several files outside the workspace asks
  for approval once for the batch, naming every target. Denying it reads none
  of the files; remembering approval covers each outside file separately. See
  [Permissions](docs/usage/PERMISSIONS.md#approval-requests).
- **Tool cards and questions.** Tool badges share an eight-cell width across
  states and themes. Ask choices use the same empty and selected marks at every
  terminal width. See [Tool activity](docs/usage/TOOL_ACTIVITY.md) and
  [Questions and plans](docs/usage/QUESTIONS_AND_PLANS.md).
- **Long commands.** The model receives clearer guidance on choosing a wait
  duration and retrieving complete command output. See
  [Command line](docs/usage/CLI.md).

## 0.6.5

Changes since 0.5.5.

- **Anthropic continuity.** Tool calls, image and PDF views, and continuations
  keep the signed message prefix stable across turns and resume. Compaction
  removes thinking tied to the replaced prefix while keeping the original
  saved history. See [Anthropic integration](docs/development/ANTHROPIC.md).
- **Larger tool requests.** Removed arbitrary item and text quotas from file
  batches, steps, plans, questions, searches and browser requests. Large reads
  save overflow for paging and report an outcome for every path. See
  [Tools](docs/usage/TOOLS.md).
- **Collaborator queue.** Additional assignments wait for a slot while up to
  eight collaborator turns run at once per conversation. See
  [Collaboration](docs/usage/COLLABORATION.md).
- **Documentation.** Guides, references, contributor procedures, maintainer
  runbooks and dated reports have separate sections, with new OpenRouter and
  local-file references. Start at the [documentation index](docs/README.md).

## 0.5.5

Changes since 0.4.5.

### Long work and context

- Jecode has one context path. Each request adds to the previous one, keeping
  earlier messages unchanged.
  When a request would pass **Auto-compact at** (default **80%**), or the
  provider rejects it as too long, the same model summarizes earlier work and
  the recent messages stay verbatim. The original history stays saved and
  readable with `history:N`. See [Long-running work](docs/usage/LONG_WORK.md).
- Project instructions and the skill list travel in one complete instruction
  snapshot instead of the start of every request. A change adds one snapshot; a
  snapshot that lists no instructions withdraws the earlier rules. The current
  snapshot is sent again after compaction or `/resume` when needed. See
  [Project instructions](docs/usage/PROJECT_INSTRUCTIONS.md#when-changes-take-effect).

### Saved conversations

- Each conversation owns one folder, `sessions/<project>/<session>/`, with its
  journal, lock, summary, complete command output (`outputs/`), temporary files
  (`tmp/`) and collaborator data (`collaboration/`). Attachments stay shared per
  project. Temporary files last as long as the conversation: they survive exit
  and resume until `/tmp clean` or deletion. See
  [Files Jecode keeps](docs/reference/FILES.md).
- Deleting a conversation keeps its lock and names any unrecognized entry left
  in its folder; retained collaborator work stays in its `collaboration/` folder.

### Removed

- Model-free records of earlier work and persistent tool-result previews.
  Long conversations are reduced only by the model summary described above.
- Checkpoints and restore: the `/checkpoints` command, the `jecode checkpoints`
  command line, the `checkpoint` tool and the `protect` argument of `bash`,
  `exec` and `process`. File changes are no longer saved for restoring; use
  your version control. `jecode checkpoints ...` is now read as a prompt.
- The project map: the `map` tool and its saved notes.
- The `edit` tool's top-level `old_text` and `new_text` arguments; it takes
  `edits` or `files`.
- Configurations containing the retired root-level `openrouter` block are
  rejected without rewriting the file. Jecode uses `selection` and `backends`; see
  [Configuration](docs/reference/CONFIGURATION.md).

### Upgrading from 0.4.5

- Saved conversations use journal format **4** with session state **2**.
  Earlier journal or session-state formats **cannot be resumed**: Jecode reports
  them as unsupported and leaves their files unchanged. The published 0.4.5
  release writes journal format 3 with session state 1; there is no migration.
  Finish or export important conversations with the version that saved them
  before updating. See [Sessions](docs/usage/SESSIONS.md#after-an-interruption).
- Conversations saved in the earlier flat layout (`sessions/<project>/ID.jsonl`
  with `outputs/`, `collaboration/` and `~/.jecode/tmp/`) are listed as using an
  earlier storage layout and cannot be resumed or deleted. Their files are
  neither moved nor removed, and attachments they reference are kept. Delete them
  yourself when you no longer need them.
- `/export` files use `jecode.conversation` format **2**, without the
  conversation-level `legacy_origin` field. Message origins are unchanged.
- Existing checkpoint archives under `checkpoints/` and project-map notes in
  `projects/<project>/map.json` in the Jecode home are neither read nor removed.
  Delete them yourself when you no longer need them.
- Explicitly saved settings are kept, including **Auto-compact at** and the
  Claude prompt-cache retention. New configurations default to 80% and five
  minutes.

### Development

- npm packaging no longer requires the retired Windows checkpoint helper or
  its disk-space API in the reviewed ABI list.
- Standalone packages omit private harness reports and require a fresh output
  directory, so removed documentation cannot remain from an earlier build.
  `PACKAGE.sh` now matches the omitted names without case, as `PACKAGE.ps1` does.
- `write` and `edit` serialize through 256 lock files in the account's temporary
  namespace. Unix accounts sharing `/tmp` use separate UID directories. If a
  lock cannot be opened or acquired, the modification fails without writing.
- Compaction compares request sizes with pending instruction snapshots,
  runtime state and continuation notes included on both sides.
- On the Claude plan, an instruction snapshot and the runtime state sent with
  it share one mid-conversation system message.
- Starting in a folder with several conversations from an earlier format
  reports them in one line; `/resume` still lists each.
- Resuming a conversation reapplies known-credential masking to saved
  instruction snapshots and runtime state as well as the summary.
- The offline context check is the read-only replay of current-format session
  journals; the synthetic record-growth probe was retired with the records.

## 0.4.5

Changes since 0.3.5.

### Long work and context

- Recognized OpenAI API models, including `gpt-6.1-sol`, use their documented
  context and output limits for the context meter, automatic reduction and the
  output allowance. Subscription limits remain account-specific; unknown models
  keep an unknown window.
- Long conversations use deterministic native records of earlier work without a
  model request, with references to the original conversation. Model summaries
  remain for when records and previews cannot make room.

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
- Process completions no longer create status-row notices, so the current
  activity and the turn's final statistics stay visible.

### Upgrading from 0.3.5

- Continue sessions with native records in **0.4.5**. Version 0.3.5
  cannot project those records, so a turn there may lack earlier work even
  though the full history stays saved. If an older version compacts a session,
  0.4.5 reads its new summary in place of stale records.
- Claude prompt caches now default to **five minutes**; one-hour retention
  remains available for breaks. Configurations saved by 0.3.5 keep its one-hour
  default. Choose **5 minutes** in `/settings` → **Context** → **Prompt cache**
  to adopt the new default. One-hour writes cost more at Anthropic's API list
  prices; see [Configuration](docs/reference/CONFIGURATION.md#prompt-cache).

### Development

- MCP inspection test servers explicitly close each HTTP connection and have a
  regression check for discovery followed by a tools list. Failed inspection
  assertions now include the panel and connection-health details.
- On macOS, resizing a PTY after its target exits now checks the watcher's exit
  state and waits for confirmed cleanup before returning the child's status.
  The watcher rechecks exit after one polling interval before reporting a live
  resize failure. Resize failures while the target is running still report an
  error. A closed PTY also stops being polled until final cleanup, avoiding a
  reproduced busy loop while preserving output and exit codes.
- Unix terminal errors identify a failed control read, resize or interrupt,
  and distinguish malformed frames and closed channels from system errors.
- GitHub Actions dependencies were updated. Offline context checks screen saved
  checkpoints and sustained record growth without provider calls; they do not
  establish a ten-hour live result. See
  [context checks](docs/development/CONTEXT_CHECKS.md) and the
  [benchmark conditions](docs/reports/AGENT_BENCHMARK_2026-11.md).

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
  [evidence for each one](docs/usage/PLATFORMS.md#supported-versions).
- **Lower token use.** A [five-turn benchmark](docs/reports/AGENT_BENCHMARK_2026-11.md)
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
  [Windows measurements](docs/reports/PERFORMANCE_RESULTS.md#provider-transports)
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
  `head` and `tail` built-ins; see [Command language](docs/reference/SHELL.md).
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
