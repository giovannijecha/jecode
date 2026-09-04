# Use Jecode

Install and start Jecode from the [project README](../README.md). This guide
covers provider access, terminal controls, durable conversations, batch mode,
configuration, and the safety boundaries that apply while Jecode works.

## Connect a provider

| Provider ID | Authentication | Notes |
| --- | --- | --- |
| `anthropic` | `ANTHROPIC_API_KEY` | Anthropic API usage |
| `openai` | `OPENAI_API_KEY` | Separately billed OpenAI API usage |
| `openai-codex` | ChatGPT OAuth | Experimental; uses eligible ChatGPT account access, not API credits |
| `ollama` | `OLLAMA_API_KEY` for cloud or remote use | Cloud with a key, local without one |

`/providers` manages access only; connecting an account or adding a key never
silently changes the runtime route. Use `/models` to choose a model. Each row
identifies whether it will use an API or the ChatGPT account, and the footer
keeps that exact route visible before and during every turn.

Choose **ChatGPT** in `/providers` to sign in on OpenAI's website without
pasting a key. Jecode supports a local browser callback and a device-code flow;
WSL and remote terminals default to the device code. Availability and usage
limits depend on the ChatGPT account and plan, not on OpenAI API credits. This
integration is experimental and is not an endorsement of Jecode by OpenAI.

Anthropic remains API-key only. Jecode does not reuse a Claude consumer
subscription or copy credentials from another client.

For Ollama, `/providers` can select cloud, local, or a custom endpoint. With an
Ollama API key Jecode defaults to `https://ollama.com`; without one it defaults
to `http://127.0.0.1:11434`. Remote custom endpoints must use HTTPS.

## Work in the TUI

Type `/` to open searchable command completion inside the composer.

### Slash commands

| Command | What it does |
| --- | --- |
| `/settings` | Manage the selected model and saved non-secret defaults |
| `/effort` | Change and save reasoning effort directly |
| `/providers` | Manage provider connections, API keys, ChatGPT sign-in, and Ollama endpoints |
| `/models` | Search all currently usable provider catalogues and select a model |
| `/permissions` | Change session tool access and review remembered approvals |
| `/timeline` | Browse resumable turns and select where the next branch should begin |
| `/compact` | Compact the current branch context without deleting saved conversation history |
| `/new` | Start a new conversation and reset session tool permissions |
| `/export` | Save a timestamped Markdown transcript in the launch directory |
| `/help` | Open a temporary keyboard reference in the composer dock |
| `/exit` | Restore the terminal and exit |

### Keyboard controls

- **Up/Down** moves through suggestions, menus, and input history.
- **Left/Right** moves the cursor or changes an inline value;
  **Ctrl+Left/Right** moves by word.
- **Backspace/Delete** removes one character;
  **Ctrl+Backspace/Delete** removes one word.
- **Home/End** moves to the beginning or end of the composer.
- **Tab** completes a slash command without running it; **Enter** sends. During
  an active model turn, **Enter** queues guidance for its next safe boundary.
- **Alt+Enter** inserts a newline.
- **Esc** closes the active menu or interrupts foreground work.
- **Ctrl+C** interrupts, or exits while idle. **Ctrl+D** requests a clean exit.
- **PageUp/PageDown** and the mouse wheel scroll the transcript.
- **Ctrl+O** expands or compacts the latest reasoning or tool-detail block.

The footer keeps the selected provider route, model, effort, and workspace
visible. During work, it adds the current state, elapsed time, steering
availability or queue count, and interrupt hint. Queued guidance joins the same
conversation turn after the provider response or complete tool batch already
in progress; `Esc` remains an immediate interruption. Operational feedback uses
the same replaceable status area instead of adding noise to the conversation or
its Markdown export.

## Resume, branch, and compact conversations

Interactive conversations are stored under `~/.jecode/sessions` and scoped to
the canonical workspace path. A fresh or `/new` conversation is not added to
the resume picker until it has a settled turn. Resuming and continuing a
conversation keeps its durable session identity and updates one picker entry
instead of creating duplicates.

`/timeline` shows completed, failed, and interrupted turns in the conversation
tree. Selecting an older turn changes only the in-memory path: it creates and
saves a branch only after the next real user message. Cancelling the picker or
exiting before that message leaves the durable head unchanged. A failed turn
keeps the same partial evidence and outcome in the live transcript, export, and
resume, while the next model receives a neutral failure boundary instead of
incomplete streamed text. Historical tools are displayed but never executed.
If a process stops abruptly inside a tool loop, Jecode resumes from the latest
safe ancestor and lets the next user turn create a branch. `/export` writes
only the currently selected path.

When model-facing context approaches the selected model's usable capacity,
Jecode asks the provider for a bounded summary of the older prefix and keeps
recent turns exact. The default trigger is 85% and can be changed from 50% to
95%. Live provider metadata or Ollama's allocated context determines the budget
when available; provider safety limits always win.

Compaction changes only the projection sent to the model. Complete messages,
tool evidence, transcript, export, and conversation tree remain intact. The
branch-local summary anchor is saved with the session so resume does not repeat
the same compaction. `/compact` requests this process immediately, even below
the automatic threshold; it leaves very small contexts unchanged. After
selecting a historical turn, send the first new message before compacting so
shared history is never rewritten.

## Run in batch mode

When stdin or stdout is not a terminal, Jecode switches to a plain
line-oriented mode:

```console
printf "explain this project\n" | jecode --root .
```

Batch conversations are stateless. Dangerous tools remain denied unless
`--auto-approve` is supplied explicitly. Terminal failures are written to
stderr and exit non-zero so scripts and CI pipelines can stop reliably.

## Configure Jecode

Startup precedence is: command-line flags, environment variables, saved
settings, then built-in defaults.

| Flag | Environment | Default |
| --- | --- | --- |
| `--provider` | `JECODE_PROVIDER` | `anthropic` |
| `--model` | `JECODE_MODEL` | Provider default or interactive selection |
| `--ollama-host` | `OLLAMA_HOST` | Cloud with an Ollama key, local without one |
| `--root` | - | Current directory |
| `--effort` | `JECODE_EFFORT` | `high` |
| `--max-tokens` | `JECODE_MAX_TOKENS` | `64000` ceiling, clamped to the usable request budget; not sent by `openai-codex` |
| `--max-steps` | `JECODE_MAX_STEPS` | No limit |
| `--compaction-percent` | `JECODE_COMPACTION_PERCENT` | `85`; accepts `50` through `95` |
| `--reduced-motion` | `JECODE_REDUCED_MOTION=1` | Off |
| `--auto-approve` | `JECODE_AUTO_APPROVE=1` | Off |
| `--ephemeral` | `JECODE_EPHEMERAL=1` | Off |

`--max-steps` is an opt-in, process-only budget for deterministic automation
and diagnostics. Interactive work has no arbitrary model-loop ceiling, and the
budget is neither shown nor saved by `/settings`.

Non-secret preferences live in `~/.jecode/settings.json`. Explicitly saved API
keys live in `~/.jecode/credentials.json`, while ChatGPT OAuth accounts live in
`~/.jecode/accounts.json`. Environment credentials always take precedence.
These JSON stores are size-bounded before parsing and writing, and invalid
fields are discarded. Secret stores use owner-only permissions where the
operating system supports them.

Jecode has one current interface theme, Slate. `NO_COLOR` is supported for
terminals and pipelines that disable colour.

## Understand the safety model

Jecode treats model output, workspace content, tool output, and terminal text as
untrusted data.

- Current filesystem tools are confined to the selected workspace. Writes
  reject symlink and junction components, revalidate boundaries and the
  approved file state immediately before atomic replacement.
- Dangerous tools ask by default unless explicitly allowed for the session or
  the process starts with `--auto-approve`.
- Credential fields are masked and excluded from transcripts. Recognized
  credential values are redacted before output reaches the model, screen,
  history, or export.
- Approved shell commands receive no secret-bearing environment variables.
  `SSH_AUTH_SOCK` is preserved so Git and SSH can use the user's agent, which
  means an approved command may ask that agent to authenticate.
- ChatGPT OAuth uses PKCE and an exact loopback callback or the OpenAI device
  flow. Refresh-token rotation is serialized across Jecode processes.
- Terminal control characters are neutralized before rendering.
- Remote Ollama endpoints require HTTPS, and provider redirects are rejected.
- Provider handshakes and idle response bodies have finite deadlines.
  Idempotent catalogue reads retry only bounded transient failures. A streaming
  generation may retry once only when its adapter identifies a transient
  pre-stream rate limit with an explicit provider delay. Billing and quota
  failures stop immediately; ambiguous failures and started streams are never
  replayed. Jecode caps the wait at 60 seconds and keeps it interruptible.
- Interactive and batch prompts are rejected above 1,048,576 UTF-16 code units
  before history or provider use. Model and filesystem input are also bounded
  before use.
- Credential redaction fails closed if its bounded supported secret set is
  exceeded.
- Session files are versioned, symmetrically size-bounded before write and
  after read, atomically checkpointed, and treated as untrusted when loaded. A
  live lease prevents concurrent resume.

`run_command` is not an operating-system sandbox. An approved command can still
access files and account resources available to the current user. Review
commands carefully and reserve `--auto-approve` for controlled environments.

Read the [security policy](../SECURITY.md) before reporting a vulnerability.
