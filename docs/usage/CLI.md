# Command line

Run Jecode from the project directory you want it to work in. That directory
becomes the workspace for tools and permissions, and saved conversations belong
to it.

```text
jecode [--plain] [--model MODEL] [--attach PATH]... [PROMPT]
jecode [--plain] resume [SESSION_ID]
jecode checkpoints [COMMAND]
jecode setup
jecode --help
jecode --version
```

## Choose an interface

| You run | You get |
| --- | --- |
| `jecode` in an interactive terminal | The fullscreen TUI. See [Terminal interface](../tui/INTERFACE.md). |
| `jecode --plain` | A line-based chat in the same terminal. |
| `jecode "PROMPT"` | One task. The answer goes to stdout and progress to stderr. |
| `jecode < task.md`, `... \| jecode` | One task read from redirected or piped stdin (up to 1 MiB). |
| `jecode resume` | A picker of this directory's saved conversations; see [Saved conversations](SESSIONS.md). |

The TUI needs both stdin and stdout to be a terminal. With redirected stdout,
or with `TERM=dumb`, use `--plain` or a single prompt instead.

## Options

| Option | Effect |
| --- | --- |
| `--plain` | Use the line-based chat instead of the TUI. It also works with `resume`. |
| `--model MODEL` | Use `MODEL` on the default provider for this run, without changing saved defaults. |
| `--attach PATH` | Attach a file to the first message. Repeat it for more files. See [Attachments](ATTACHMENTS.md). |
| `-h`, `--help` | Print usage, commands and keys. Needs no configuration. |
| `-V`, `--version` | Print the version and build identity. Needs no configuration. |

Options go before the prompt. Any argument that starts with `-` is read as an
option, so put `--` before a prompt that starts with `-` or contains a separate
argument that does, and before a prompt whose first word is `resume` or
`checkpoints`. Quoting the whole prompt as one argument avoids most of these
cases:

```sh
jecode -- "-v flag behaves differently on Windows"
jecode -- resume the interrupted migration
```

`setup`, `--help` and `--version` are recognized only as the sole argument.

## Single prompts and pipes

```sh
jecode "Explain the structure of this project"
jecode --model provider/model-id "Review the last change in src/config.rs"
jecode --attach report.pdf "Summarize this report"
jecode < task.md
{ echo "Review this diff:"; git diff; } | jecode
```

Replace `provider/model-id`, `report.pdf` and `task.md` with your own model ID
and files. The last two lines are POSIX shell syntax; in PowerShell, use
`Get-Content task.md -Raw | jecode`.

Piped stdin is the task only when there is no prompt argument; with a prompt
argument, stdin is ignored. Put the instruction and the data together in the
piped text, as above. Windows PowerShell 5.1 encodes text piped to a program
with `$OutputEncoding`, which defaults to ASCII; for non-ASCII input, run
`$OutputEncoding = [Text.UTF8Encoding]::new()` first.

To continue a saved conversation non-interactively, pipe the next message into
`jecode resume SESSION_ID`.

A single prompt runs one complete turn, including any tool calls, then exits.
It saves a conversation you can continue with `jecode resume`. Two limits apply
because nobody is there to answer:

- Calls that need [permission approval](PERMISSIONS.md#approval-requests) are
  refused, and [questions](QUESTIONS_AND_PLANS.md) return to the model as
  unavailable.
- A pipe never runs setup. Without configuration it fails with
  `Jecode is not configured. Run jecode setup in a terminal, then retry your task.`

In an interactive terminal, a single prompt whose provider has no credentials
starts setup first.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success. |
| `1` | The task or command failed; the error is printed on stderr. |
| `2` | Invalid arguments, for example an unknown option. |

## Plain chat

`jecode --plain` reads one message per line and prints answers as plain text.
Enter on an empty line sends only staged attachments. It supports these
commands:

| Command | Purpose |
| --- | --- |
| `/new` | Start a new conversation. |
| `/resume [ID]` | Choose, resume or delete a saved conversation (`d NUMBER` or `d ID`, then type `delete`). |
| `/model [ID]`, `/effort [NAME]`, `/routing` | Change the model, effort or OpenRouter endpoint for this conversation. |
| `/permissions [read\|workspace\|full]` | Show or change the permission level. `/permissions remembered` lists approvals; `/permissions revoke N` removes one. |
| `/settings` | Run guided setup. |
| `/init` | Create or update the project's `AGENTS.md`. |
| `/skills` | List personal skills. |
| `/copy` | Copy part of the latest answer through a numbered menu. |
| `/export` | Save the conversation as JSON. |
| `/tmp [clean]` | Show or clear the conversation's temporary files. |
| `/checkpoints [COMMAND]` | Inspect or restore checkpoints. |
| `/terminal [ID]` | List background processes or show one's status and last output. |
| `/terminal stop ID` | Stop a process. |
| `/terminal write ID TEXT`, `/terminal interrupt ID`, `/terminal resize ID COLS ROWS` | Type a line into, interrupt or resize an interactive process. |
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
- There is no `/ask`; questions are asked inline.

## Checkpoint commands

`jecode checkpoints` inspects and restores [checkpoints](CHECKPOINTS.md) without
a provider, key or network:

```text
jecode checkpoints list [--limit N]
jecode checkpoints inspect rc-ID [PATH...]
jecode checkpoints show rc-ID PATH [--offset N | --byte-offset N] [--limit N]
jecode checkpoints restore rc-ID [PATH...] [--preview] [--expected TOKEN|latest]
```

Run `jecode checkpoints help` for the same summary.
