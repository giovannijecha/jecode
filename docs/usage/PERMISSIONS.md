# Permissions

Every conversation has a permission level that decides what the agent's tools
may do. You choose the level; the model cannot raise it or approve its own
requests. Jecode enforces the level on each tool call.

## Levels

The working folder is the directory you started Jecode in, with all its
subdirectories.

| Level | In the working folder | Outside it |
| --- | --- | --- |
| **Read** | Read and search files; fetch web pages with GET or HEAD. No edits, commands, checkpoint restores or browser control. | Refused. |
| **Workspace** (default) | Read and edit files; run commands, confined on Windows (see [Platform limits](#platform-limits)). Network access is allowed. | Each file access, and each command started from an outside directory, asks for [approval](#approval-requests). |
| **Full** | Everything, with your normal user permissions. | Everything, with your normal user permissions. |

Levels govern Jecode's tool calls. Once a command runs, it acts with whatever
authority it was given; the working folder is not a sandbox on its own. The
network is reachable in Workspace and Full, and no level restricts which hosts
can be contacted.

Read still lets Jecode save the conversation and use its own temporary storage.
It never asks for permission to edit: switch levels instead.

## Choose a level

| Action | Scope |
| --- | --- |
| `/permissions` | Choose this conversation's level, or open **Remembered** approvals. |
| `/permissions read`, `/permissions workspace`, `/permissions full` | Change this conversation's level directly. |
| `/settings` → **Default permissions** | Save the level new conversations start at. |
| `permissions.default` in [`config.json`](CONFIGURATION.md#the-configuration-file) | The same default, edited by hand. |

The TUI shows the current level as a chip on the composer's lower border. A
level change applies to later tool calls. Calls already running, including
background processes, keep the authority they started with; stop a process from
`/terminal` if needed. Resume restores the conversation's level and approvals;
`/new` starts at the saved default.

## Approval requests

In Workspace, a file call outside the working folder, or an `exec` call with an
outside working directory, waits for you. The request shows which agent is
asking, the target and the kind of access:

1. **Approve** runs this call once.
2. **Approve and remember** also allows the same access for the rest of this
   conversation, including after resume. File and folder approvals match their
   exact path. Command approvals match the exact program, arguments, explicit
   environment, input and terminal settings; no command prefix is approved.
3. **Deny** runs nothing and tells the agent.

In the TUI, type `1`, `2` or `3`, or move with the arrow keys and press Enter.
Nothing is selected when the request appears, so a stray Enter does nothing, and
pasted text never answers it. If the request arrives while you are typing, your
keys keep going to the draft until you pause or press an arrow key or Tab.
Ctrl+C stops the turn. Requests appear one at a time.

Approving never switches the conversation to Full. To review approvals, open
`/permissions` → **Remembered** and press Ctrl+D on one to revoke it. Read and
Full pause remembered Workspace approvals without deleting them.

Collaborators share the conversation's level and approvals, and their requests
name the collaborator. In `--plain`, the main agent's requests appear as
numbered questions and closing stdin denies them; collaborators cannot ask
there. Single prompts and pipes cannot ask at all, so calls that need approval
are refused.

## Command confinement

On Windows, Workspace runs `bash` and non-interactive `exec` commands inside a
Windows Process Security Environment. The operating system enforces a
filesystem policy on the command and every process it starts:

- The working folder and the conversation's temporary area are writable.
- Installed Rust toolchain data is readable, and Cargo's cache is redirected to
  the temporary area.
- The rest of the Jecode home, including configuration, credentials and other
  conversations, is outside the writable area.
- Commands inherit only a fixed set of operating-system and toolchain
  environment variables, not arbitrary tokens from your shell. Values set
  explicitly in an `exec` call still apply.

If the Jecode home lies inside the working folder, Workspace refuses to run
commands. Start Jecode in a narrower project directory, or choose Full.

## Platform limits

- **Linux and macOS have no command confinement.** In Workspace, every `bash`
  and `exec` call fails with: `Workspace command confinement is supported only
  on Windows. Choose /permissions full to run without confinement on this
  platform.` File tools still follow the Workspace rules. To let the agent run
  commands, choose Full for the conversation (`/permissions full`) or as the
  default.
- **Windows hosts without the required support** (Process Security Environment
  1.1 with filesystem and network policies) also refuse Workspace commands.
  There is no automatic fallback to Full on any platform.
- **Interactive terminals** (`exec` with `tty: true`) require Full on every
  platform.
- **Browser control** requires Full, because browser file access and downloads
  are not confined. Closing or stopping a browser the conversation already
  started works at any level. In Read and Workspace, use the `web` tool for
  network retrieval.
- **Local servers on Windows.** A confined command can connect to a server on
  your machine, but your machine cannot connect to a server running inside the
  confinement. Start development servers in Full; you can return to Workspace
  afterwards and the server keeps running.
- **Windows services and CI workers.** In a service session (session 0), Windows
  cannot start some programs, including PowerShell, inside the confinement. Run
  such programs from an interactive user session, or choose Full. See the
  [Windows issue](https://github.com/microsoft/mxc/issues/1465).

## What permissions do not cover

Permissions do not isolate the whole Jecode process. File tools check paths
before they act, and Windows confinement covers command processes; network
services you reach act with their own authority. Full is unconfined, and
confinement does not undo changes made inside the working folder. Use
[checkpoints](CHECKPOINTS.md) to recover files after an unwanted change.

Conversations save level changes, approvals, revocations and decisions. A
request that was pending when Jecode stopped is recorded as not executed on
resume; it is never replayed automatically.
