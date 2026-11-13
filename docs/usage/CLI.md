# Command line

Run Jecode from the project directory you want it to work in, or name that
directory with `-C`. That directory becomes the workspace for tools and
permissions, and saved conversations belong to it.

```text
jecode [OPTIONS] [--] [PROMPT]
jecode [OPTIONS] resume [--last | SESSION_ID] [--] [PROMPT]
jecode init [OPTIONS]
jecode setup
jecode settings   [show | get KEY | set KEY VALUE | path]
jecode providers  [list | accounts PROVIDER | use PROVIDER ACCOUNT_ID]
jecode providers  login PROVIDER
jecode providers  logout PROVIDER [--account ACCOUNT_ID] [--yes]
jecode models     [list | show ID | endpoints ID]
jecode sessions   [list | show ID | export ID | delete ID]
jecode skills     [list | show NAME]
jecode mcp        [list | show | tools | add | login | enable | disable | remove]
jecode lsp        [list | show | add | check | enable | disable | remove]
jecode doctor
jecode completions bash|zsh|fish|powershell
jecode update [--check]
jecode uninstall
jecode help [COMMAND]    jecode COMMAND --help    jecode --version
```

`jecode --help` lists the commands; `jecode help COMMAND` or
`jecode COMMAND --help` shows a command's usage, options and subcommands. Help
never runs the command, starts setup or contacts a provider. Every command and
option is listed in [Commands and keys](../reference/COMMANDS.md#command-line).

## Choose an interface

| You run | You get |
| --- | --- |
| `jecode` in an interactive terminal | The fullscreen TUI. See [Terminal interface](INTERFACE.md). |
| `jecode --plain` | A line-based chat in the same terminal. |
| `jecode "PROMPT"` | One task. The answer goes to stdout and progress to stderr. |
| `jecode < task.md`, `... \| jecode` | One task read from redirected or piped stdin. |
| `jecode resume` | A picker of this directory's saved conversations; see [Saved conversations](SESSIONS.md). |
| `jecode resume --last "PROMPT"` | One more message in the most recent conversation, then exit. |

The TUI needs both stdin and stdout to be a terminal. With redirected stdout,
or with `TERM=dumb`, use `--plain` or a single prompt instead.

## Run options

These apply to the TUI, plain chat, single prompts, `resume` and `init`
(`init` takes the provider, model, effort, routing, permission and JSON options).

| Option | Effect |
| --- | --- |
| `-C DIR`, `--cd DIR` | Work in `DIR` instead of the current directory. Works with every command. |
| `--plain` | Use the line-based chat instead of the TUI. |
| `--provider ID` | Use this provider for the conversation: `openrouter`, `openai-api` or `anthropic-api` (API keys, API billing), `openai` (ChatGPT plan) or `anthropic` (Claude plan). |
| `--model ID` | Use model `ID` for the conversation. |
| `--effort NAME` | Use this reasoning effort: `default`, `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max` or `ultra`. |
| `--routing auto\|ENDPOINT` | Use automatic routing or an OpenRouter endpoint for this conversation. |
| `--permissions LEVEL` | Start the conversation at `read`, `workspace` or `full`. See [Permissions](PERMISSIONS.md). |
| `--attach PATH` | Attach a file to the first message. Repeat it for more files. See [Attachments](ATTACHMENTS.md). |
| `--json` | Print the run as JSON Lines; see [Structured output](#structured-output). |
| `-h`, `--help` | Show help for the command. Needs no configuration. |
| `-V`, `--version` | Print the version and build identity. Needs no configuration. |

Run options never change saved defaults. Change those with
[`jecode settings`](../reference/CONFIGURATION.md#change-settings) or
`/settings`.

### Provider, model, effort and permissions

What a conversation uses is decided in this order:

1. A run option you supply, for this conversation only.
2. For `resume`, the conversation's own saved provider, model, effort, routing
   and permission level.
3. Your saved defaults in `config.json`.
4. `OPENROUTER_API_KEY` and `OPENROUTER_MODEL`, only when no `config.json` exists.

More precisely:

- `--provider` never switches to another billing or account route. If that
  provider has no saved key or active account, the run stops and names
  `jecode providers login PROVIDER`; it does not fall back.
  An explicit provider and model can use a saved account even when
  `config.json` has not been created; no default is saved implicitly.
- `--provider` without `--model` uses that provider's saved model; if it has
  none, the run asks for `--model`. `--model` without `--provider` keeps the
  provider: the default one for a new conversation, the conversation's own when
  resuming.
- A model other than the one an effort was saved with starts at `default`
  effort unless you give `--effort`. On OpenRouter and Anthropic, an effort the
  model's catalog does not list is refused before the run starts.
- `--routing` overrides the conversation's OpenRouter endpoint without changing
  saved defaults. It also works with `resume`; an endpoint must be available
  for the chosen model. List IDs and compatibility with
  `jecode models endpoints MODEL --provider openrouter`. Other providers reject
  this option.
- When resuming, the options change that conversation as `/model`, `/effort`
  and `/permissions` would; the conversation records a `Run options` entry with
  the old and new values. This also applies after choosing a conversation in
  the TUI resume picker. An invalid explicit override stops the launch rather
  than opening the conversation with its previous choices.
- In the TUI, if the default provider has no credentials, Jecode may open a
  new conversation with another configured provider and says so in a notice.
  It never does this for `--provider`, and never for single prompts.

## Grammar

- The first argument that exactly matches a command name selects that command.
  Anything else starts the prompt, so `jecode fix the parser` sends
  `fix the parser`. A longer prompt can be quoted as one argument
  (`jecode "update the README"`). Use `--` for a prompt that is exactly a
  command name (`jecode -- resume`); quoting that name alone does not change
  the argument Jecode receives.
- Options precede the prompt. For other commands they may appear anywhere
  after the command name. An argument starting with `-` after the prompt has
  begun is an error; put `--` before a prompt that contains one.
- Options take their value as the next argument or after `=`:
  `--model ID` or `--model=ID`. Write `--model=-x` for a value that starts
  with `-`. Short options (`-C`, `-h`, `-V`) take a separate value.
- Only `--attach` may repeat. Unknown options, options a command does not
  accept, invalid choices, missing values and the wrong number of arguments are
  rejected before anything runs, with exit status 2 and the command's help
  pointer on stderr.
- `--` ends option parsing; everything after it is the prompt.
- `-h` or `--help` anywhere before `--` shows help instead of running anything.
- Arguments must be valid UTF-8.

```sh
jecode -- "-v flag behaves differently on Windows"
jecode "resume the interrupted migration"
jecode -C ../service --model provider/model-id "Review the last change"
```

## Single prompts and pipes

```sh
jecode "Explain the structure of this project"
jecode --attach report.pdf "Summarize this report"
jecode < task.md
git diff | jecode "Review this diff"
```

Replace `report.pdf` and `task.md` with your own files. The pipes are POSIX
shell syntax; in PowerShell, use `Get-Content task.md -Raw | jecode` or
`git diff | jecode "Review this diff"`.

When stdin is not a terminal, Jecode reads it before the run starts:

| Prompt argument | Piped stdin | The message |
| --- | --- | --- |
| No | Text | The piped text is the task. |
| No | Empty or only whitespace | An error: `No task`. |
| Yes | Text | The prompt, a blank line, then the data between `<stdin>` and `</stdin>` lines, verbatim. |
| Yes | Empty | The prompt alone. |

Piped input is limited to 1 MiB and must be UTF-8; a leading UTF-8 byte order
mark is dropped. Larger or non-UTF-8 input stops the run with an error before
any conversation is created; nothing is truncated or silently discarded.
Windows PowerShell 5.1 encodes text piped to a program with `$OutputEncoding`,
which defaults to ASCII; for non-ASCII input, run
`$OutputEncoding = [Text.UTF8Encoding]::new()` first.

Because stdin that is not a terminal is always read, a run waits until it ends.
When a script or tool leaves stdin open, redirect it from an empty source:
`< /dev/null` on Linux and macOS, `< NUL` in `cmd.exe`, or pipe
`$null |` in PowerShell.

A single prompt runs one complete turn, including any tool calls, then exits.
It saves a conversation you can continue later. Because nobody is there to
answer:

- Calls that need [permission approval](PERMISSIONS.md#approval-requests) are
  refused, and [questions](QUESTIONS_AND_PLANS.md) return to the model as
  unavailable.
- A pipe or `--json` run never starts setup. Without configuration or an
  explicitly selected configured provider and model, it fails
  with `Jecode is not configured. Run jecode setup in a terminal, then retry your task.`

In an interactive terminal, a single prompt whose default provider has no
credentials starts setup first.

## Continue a conversation

```sh
jecode resume --last "Now add the missing test"
jecode resume 1793721600123-4812-0 "Explain the failure"
git log -5 | jecode resume --last "Summarize these commits"
```

`--last` picks the most recent conversation of this directory. With a prompt or
piped stdin, the message is sent and Jecode exits; the same stdin rules apply.
Without them, the TUI or plain chat opens on the conversation. `jecode sessions
list` shows the IDs.

## Initialize a project

`jecode init` runs the same model task as `/init`: the model inspects the
project and creates or updates the root `AGENTS.md`. It is a single run, so
permission requests are refused; the default Workspace level allows the edit.
See [Project instructions](PROJECT_INSTRUCTIONS.md).

## Structured output

`--json` makes output machine-readable on stdout. Text for people, progress and
errors stay on stderr.

- Management commands (`settings`, `providers`, `models`, `sessions`, `skills`,
  `mcp`, `lsp`, `doctor`, `update`, `uninstall`) print one JSON document.
- Runs print JSON Lines, one object per line with a `type`: `message`,
  `tool_started`, `tool_finished`, `tool_note`, `process_finished`,
  `collaboration`, `retrying`, `backend_failure`, `context` and `usage`. Once
  the run has started, the last line is always `result`, with `status`
  (`completed` or `failed`), `session_id`, `provider`, `model`, `effort`,
  `permissions` and, on failure, `error`. A failure before the run starts, such
  as missing configuration, prints nothing on stdout. A run with `--json` needs
  a prompt, piped stdin or `init`.

Credentials never appear in either form: saved keys, account tokens and MCP or
LSP secrets are redacted, as in the TUI.

## Manage Jecode from the shell

| Command | Purpose | Interactive equivalent |
| --- | --- | --- |
| `jecode setup` | Guided provider, account and model configuration. | `/settings` |
| `jecode settings` | Show or change saved defaults; `settings path` prints the configuration file. | `/settings` |
| `jecode providers` | Provider status, account listing and selection, `login` and `logout`. | Settings → Providers |
| `jecode models` | List or show a provider's models. | `/model` |
| `jecode sessions` | List, show, export or delete this folder's conversations. | `/resume`, `/export` |
| `jecode skills` | List or show personal skills. | `/skills` |
| `jecode mcp` | Inspect, add, sign in, enable, disable or remove MCP connections. | `/mcp`, Settings → MCP |
| `jecode lsp` | Inspect, add, check, enable, disable or remove language servers. | `/lsp`, Settings → LSP |
| `jecode doctor` | Read-only diagnostics. | — |
| `jecode update`, `jecode uninstall` | Update or remove this installation. | — |

Inspection (`settings`, `providers list` and `accounts`, `sessions list` and `show`, `skills`,
`mcp list` and `show`, `lsp list` and `show`, `doctor`) works without model
credentials. It opens no conversation, changes no configuration or session
content, asks no model, starts no language server and signs nothing in.
Existing credential readers may reapply account-file permissions as part of
their protection policy. `models`, `mcp tools`,
`mcp add`, `mcp login`, `lsp check`, `providers login` and `update` contact a
service or start a program because that is what you asked for.

Commands that delete or remove ask you to type a word to confirm. Without a
terminal they never wait: add `--yes`. Keys and tokens are read from a hidden
prompt or from piped stdin, never from arguments:

```sh
jecode providers login openrouter            # hidden prompt
printf '%s' "$KEY" | jecode providers login openrouter
```

The [Shell tab of `/help`](../reference/COMMANDS.md#keys) lists these commands
inside Jecode.

## Shell completion

`jecode completions SHELL` prints a completion script for `bash`, `zsh`, `fish`
or `powershell`:

```sh
source <(jecode completions bash)            # ~/.bashrc
source <(jecode completions zsh)             # ~/.zshrc, after compinit
jecode completions fish | source             # ~/.config/fish/config.fish
```

In PowerShell, add `jecode completions powershell | Out-String | Invoke-Expression`
to your profile.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success. |
| `1` | The task or command failed; the error is printed on stderr. `jecode doctor` also exits 1 when a check fails. |
| `2` | Invalid usage, for example an unknown option, a missing argument or a needed `--yes`. Nothing ran. |

## Plain chat

`jecode --plain` reads one message per line and prints answers as plain text.
Enter on an empty line sends only staged attachments. Messages, here and in a
single prompt, can name a skill as `$name` or a project file as `@path`, written
as the TUI's selectors write them; see
[Reference skills and files](INTERFACE.md#what-a-reference-means). It supports these
commands:

| Command | Purpose |
| --- | --- |
| `/new` | Start a new conversation. |
| `/resume [ID]` | Choose, resume or delete a saved conversation (`d NUMBER` or `d ID`, then type `delete`). |
| `/model [ID]`, `/effort [NAME]`, `/routing` | Change the model, effort or OpenRouter endpoint for this conversation. |
| `/permissions [read\|workspace\|full]` | Show or change the permission level. `/permissions remembered` lists approvals; `/permissions revoke N` removes one. |
| `/settings` | Run guided setup. Other saved defaults change with `jecode settings`. |
| `/init` | Create or update the project's `AGENTS.md`. |
| `/skills` | List personal skills. |
| `/mcp` | List configured MCP connections. Manage them with `jecode mcp`. |
| `/lsp` | Inspect configured language servers and observed conversation instances, without starting a server. Manage them with `jecode lsp`. |
| `/copy` | Copy part of the latest answer through a numbered menu. |
| `/export` | Save the conversation as JSON. |
| `/tmp [clean]` | Show or clear the conversation's temporary files. |
| `/process [ID]` | List running and recent processes, or show one's status and last output. |
| `/process stop ID` | Stop a process. |
| `/process write ID TEXT`, `/process interrupt ID`, `/process resize ID COLS ROWS` | Type a line into, interrupt or resize an interactive process. |
| `/agents [NUMBER]` | List collaborators, or show one. |
| `/todo` | Show the current plan. |
| `/help` | Show commands. |
| `/exit` | Quit. |

Differences from the TUI:

- Permission requests and questions are asked as numbered prompts. Collaborators
  cannot ask; their requests are refused. Closing stdin denies a pending request.
- Plain chat does not watch collaborators while it waits for your input, so a
  returned collaborator does not wake the main agent; live inspection and
  automatic continuation are TUI features.
- A process that ends while plain chat waits for input appears at the next line you
  enter. Its outcome is saved by then, or when you leave the conversation.
- There is no `/ask`; questions are asked inline.
