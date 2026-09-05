# Use Jecode

Install and start Jecode from the [project README](../README.md). This guide
covers provider access, terminal controls, durable conversations, startup options,
configuration, and the safety boundaries that apply while Jecode works.

## Connect a provider

| Provider ID | Authentication | Notes |
| --- | --- | --- |
| `anthropic` | `ANTHROPIC_API_KEY` | Anthropic API usage |
| `openai` | `OPENAI_API_KEY` | Separately billed OpenAI API usage |
| `openai-codex` | OpenAI Account via OAuth | Experimental; uses eligible ChatGPT account access, not API credits |
| `ollama` | `OLLAMA_API_KEY` | Ollama API at `https://ollama.com`; cloud only |

`/providers` manages access only; connecting an account or adding a key never
silently changes the runtime route. Use `/models` to choose a model. Each row
identifies whether it will use an API or the OpenAI Account, and the footer
keeps that exact route visible before and during every turn.

Choose **Account → OpenAI Account** in `/providers` to sign in on OpenAI's website without
pasting a key. Jecode supports a local browser callback and a device-code flow;
WSL and remote terminals default to the device code. Availability and usage
limits depend on the ChatGPT account and plan, not on OpenAI API credits. This
integration is experimental and is not an endorsement of Jecode by OpenAI.

Anthropic remains API-key only. Jecode does not reuse a Claude consumer
subscription or copy credentials from another client.

The **API** group lists **Anthropic API**, **OpenAI API**, and **Ollama API**.
Each uses the same key-management flow. Ollama connects directly to the
[official cloud API](https://docs.ollama.com/cloud#cloud-api-access) and always
requires an API key. Local models and custom endpoints are not supported.

## Work in the TUI

Type `/` to open searchable command completion inside the composer.

Sent messages have a padded background without a leading symbol. With
`NO_COLOR`, their inset and surrounding space still separate them from replies.
Your text keeps Markdown punctuation, indentation, and explicit line breaks;
long lines wrap to the terminal width without changing the saved message.

### Slash commands

| Command | What it does |
| --- | --- |
| `/settings` | Manage the selected model and saved non-secret defaults |
| `/effort` | Change and save reasoning effort directly |
| `/providers` | Manage API keys and OpenAI Account sign-in |
| `/models` | Search all currently usable provider catalogues and select a model |
| `/permissions` | Change session tool access and review remembered approvals |
| `/timeline` | Browse resumable turns and select where the next branch should begin |
| `/compact` | Compact the current branch context without deleting saved conversation history |
| `/new` | Start a new conversation and reset session tool permissions |
| `/export` | Save a timestamped Markdown transcript in the launch directory |
| `/help` | Open a temporary keyboard reference in the composer dock |
| `/exit` | Restore the terminal and exit |

Commands and menus remain available while the model works. Model, effort,
output-limit, and compaction changes apply to the next turn; the current turn
keeps its original selection, including queued guidance and automatic compaction.
Reduced motion takes effect immediately. Permission changes govern calls that
have not started; enabling a tool omitted from the current turn's catalogue
makes it available on the next turn. Running tools finish under their existing
approval unless interrupted.

An approval temporarily takes focus from an open menu and returns to it after
the answer. `/new`, `/timeline`, and `/compact` require a settled conversation;
during a turn they retain the command in the composer with a short notice.
`/export` saves a snapshot of the transcript at invocation. `/exit` interrupts
work and waits for settlement before exiting.

### Keyboard controls

- **Up/Down** moves through suggestions, menus, and input history.
- **Left/Right** moves the cursor or changes an inline value;
  **Ctrl+Left/Right** moves by word.
- **Backspace/Delete** removes one character;
  **Ctrl+Backspace/Delete** removes one word.
- **Home/End** moves to the beginning or end of the composer.
- **Tab** completes a slash command without running it; **Enter** runs the
  command or sends text. During an active model turn, ordinary text queues
  guidance for its next safe boundary.
- **Alt+Enter** inserts a newline.
- **Esc** closes the active menu or interrupts foreground work.
- **Ctrl+C** cancels the active menu or command, interrupts the model when the
  composer is free, or exits while idle. **Ctrl+D** requests a clean exit.
- **PageUp/PageDown** and the mouse wheel scroll the transcript.
- **Ctrl+O** expands or compacts the latest reasoning or tool-detail block.

Menus mark the selected row with `●` and a subtle background. Labels and values
carry the choice without an explanatory line below the list. Narrow terminals
can use up to two stable overflow rows for clipped selected labels or values.
The model selector opens directly on its filter and choices; actual catalogue
failures appear as transient footer feedback.
Search and writable fields retain the `→` input prompt.
Approvals show the question and target separately. **Enter** confirms the
highlighted choice; **Y** approves once, **A** remembers the displayed scope
for this session, and **N** or **Esc** refuses with feedback.

Assistant text, reasoning, and tool records use the full terminal width and
share the composer's left edge. Each tool names its target and execution state,
with connected evidence below. Compact diffs show six changed lines shared between
the beginning and end, with a count of omitted changes. Command previews show
up to four output rows; failures retain a diagnostic line alongside the tail
when one is recognized. Ctrl+O reveals the complete retained source, including
unchanged diff context. These previews never shorten saved history or export.

When asynchronous terminal output falls behind, Jecode combines pending display
updates into the current frame as output becomes available. Conversation history
remains complete, and input and menu state continue to update.

The footer keeps the running turn's provider route, model, effort, and workspace
visible, then shows the next selection after settlement. During work, it adds the current state, elapsed time, steering
availability or queue count, and interrupt hint. Queued guidance joins the same
conversation turn after the provider response or complete tool batch already
in progress; with a menu closed, `Esc` interrupts immediately. Operational feedback uses
the same replaceable status area instead of adding noise to the conversation or
its Markdown export.

## Resume, branch, and compact conversations

Run `jecode -c` (or `jecode resume --last`) to continue the most recently updated
available conversation in the current workspace. Run `jecode resume` to choose
from the searchable session picker. Both require an interactive terminal and
cannot be combined with `--ephemeral`. The former `--latest` spelling has been
replaced by `--last`.

Interactive conversations are stored under `~/.jecode/sessions` and scoped to
the canonical workspace path. A fresh or `/new` conversation is not added to
the resume picker until it has a settled turn. Resuming and continuing a
conversation keeps its durable session identity and updates one picker entry
instead of creating duplicates.

`/timeline` shows completed, failed, and interrupted turns in the conversation
tree, with one row per turn. Long previews truncate within their row; selection
does not repeat them below the list or reserve extra space.
Selecting an older turn changes only the in-memory path: it creates and
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
95% in `/settings`. Live provider model metadata determines the budget
when available; provider safety limits always win.

Compaction changes only the projection sent to the model. Complete messages,
tool evidence, transcript, export, and conversation tree remain intact. The
branch-local summary anchor is saved with the session so resume does not repeat
the same compaction. `/compact` requests this process immediately, even below
the automatic threshold; it leaves very small contexts unchanged. After
selecting a historical turn, send the first new message before compacting so
shared history is never rewritten.

## Start from an interactive terminal

Jecode requires a terminal on both stdin and stdout. Start it directly without
pipes or redirection. If either stream is not a terminal, startup reports an
error on stderr and exits non-zero before reading configuration, selecting a
provider, or opening session storage. Batch execution is no longer available.

`jecode --help` and `jecode --version` work through pipes and redirection. These
information requests finish before configuration and terminal checks.

## Configure Jecode

Startup options select the workspace, conversation persistence, and reduced
motion:

| Flag | Environment | Default |
| --- | --- | --- |
| `--root` | - | Current directory |
| `--reduced-motion` | `JECODE_REDUCED_MOTION=1` | Off |
| `--ephemeral` | `JECODE_EPHEMERAL=1` | Off |

For reduced motion, the flag overrides the environment, then the saved setting,
then the default. Ephemeral mode uses its flag, then its environment variable;
it is not saved. Both flags accept explicit `true` or `false` values.

Configure model requests inside Jecode. Saved choices apply on later launches:

| Preference | Control | Default |
| --- | --- | --- |
| Provider and model | `/models`, or the model row in `/settings` | Anthropic API with its default model |
| Reasoning effort | `/effort` or `/settings` | `high` |
| Maximum output tokens | `/settings` | `64000` ceiling, clamped to the usable request budget; not sent by `openai-codex` |
| Context compaction | `/settings` | `85%`; accepts `50%` through `95%` |

Reasoning levels are `low`, `medium`, `high`, `xhigh`, and `max`. Each provider
exposes the subset supported by the selected model.

### Replace retired startup overrides

The following flags and environment overrides are no longer supported:

| Retired flags | Retired environment variables | Replacement |
| --- | --- | --- |
| `--provider`, `--model` | `JECODE_PROVIDER`, `JECODE_MODEL` | `/models` |
| `--effort` | `JECODE_EFFORT` | `/effort` |
| `--max-tokens`, `--compaction-percent` | `JECODE_MAX_TOKENS`, `JECODE_COMPACTION_PERCENT` | `/settings` |
| `--auto-approve` | `JECODE_AUTO_APPROVE` | `/permissions` for session tool policies |
| `--max-steps` | `JECODE_MAX_STEPS` | Ordinary turns have no model-request ceiling |

Remove retired flags from launch commands and shortcuts. A nonempty retired
environment variable stops startup: the error names the variable and gives
removal guidance without printing its value. Remove it from the current shell
and any shell profile or system environment that sets it, then configure Jecode
through the controls above. API-key environment variables continue to work.
Saved settings and conversations require no migration.

`--ollama-host` has been removed. Old `OLLAMA_HOST` and saved `ollamaHost`
values are accepted only when they name the official cloud URL; they no longer
configure an endpoint. Any other value stops startup with a removal message,
so a former local or custom connection is never silently redirected to cloud.
Remove the retired value explicitly to continue. Existing files remain intact;
an old cloud setting disappears on the next successful settings save.

### Local settings and accessibility

Non-secret preferences live in `~/.jecode/settings.json`. Explicitly saved API
keys live in `~/.jecode/credentials.json`, while ChatGPT OAuth accounts live in
`~/.jecode/accounts.json`. Environment credentials always take precedence.
These JSON stores are size-bounded before parsing and writing. Startup tolerates
damaged state; saving rejects unknown, malformed, or oversized stored data
without rewriting it. Secret stores use owner-only permissions where the
operating system supports them.

Jecode has one current interface theme, Slate. `NO_COLOR` disables semantic
colour in the terminal. `--reduced-motion` disables the travelling light on
active tool connectors and keeps the input cursor
steady; provider text continues to stream. Output text, expanded evidence,
waiting calls, and completed calls stay still.

## Understand the safety model

Jecode treats model output, workspace content, tool output, and terminal text as
untrusted data.

- Current filesystem tools are confined to the selected workspace. Writes
  reject symlink and junction components, revalidate boundaries and the
  approved file state immediately before atomic replacement.
- Dangerous tools ask by default. Approve one call, remember its displayed scope,
  or change a tool's session policy through `/permissions`. Policies remain
  adjustable, and `/new` resets them and clears remembered approvals.
- Credential fields are masked and excluded from transcripts. Recognized
  credential values are redacted before output reaches the model, screen,
  history, or export.
- Approved shell commands receive no secret-bearing environment variables.
  `SSH_AUTH_SOCK` is preserved so Git and SSH can use the user's agent, which
  means an approved command may ask that agent to authenticate.
- ChatGPT OAuth uses PKCE and an exact loopback callback or the OpenAI device
  flow. Refresh-token rotation is serialized across Jecode processes.
- Terminal control characters are neutralized before rendering.
- Model requests use fixed HTTPS provider endpoints, and redirects are rejected.
- Provider handshakes and idle response bodies have finite deadlines.
  Idempotent catalogue reads retry only bounded transient failures. A streaming
  generation may retry once only when its adapter identifies a transient
  pre-stream rate limit with an explicit provider delay. Billing and quota
  failures stop immediately; ambiguous failures and started streams are never
  replayed. Jecode caps the wait at 60 seconds and keeps it interruptible.
- Prompts are rejected above 1,048,576 UTF-16 code units
  before history or provider use. Model and filesystem input are also bounded
  before use.
- Credential redaction fails closed if its bounded supported secret set is
  exceeded.
- Session files are versioned, symmetrically size-bounded before write and
  after read, atomically checkpointed, and treated as untrusted when loaded. A
  live lease prevents concurrent resume.

`run_command` is not an operating-system sandbox. An approved command can still
access files and account resources available to the current user. Review
commands and remembered approval scopes carefully.

Read the [security policy](../SECURITY.md) before reporting a vulnerability.
