# Permissions

Every conversation has a permission level that decides what the agent's tools
may do. You choose the level; the model cannot raise it or approve its own
requests. Jecode enforces the level on file and command calls, web requests
other than GET or HEAD, browser control and collaborator integration.

[MCP](MCP.md) calls are not governed by Read, Workspace or Full. Each
connection's global enabled or suspended state, and the permissions granted at
the service, govern its calls at every level, including Read. Suspend a
connection in Settings → **MCP** in the TUI, or in `~/.jecode/mcp.json`, to
block future calls globally. Server hints do not grant permissions.

[LSP](LSP.md) calls likewise follow explicit global server enablement at every
local level. Enabled language servers run as your user with their configured
runtime/toolchain behavior. Their root is analysis context, not filesystem or
network confinement. Turn Off in Settings → LSP blocks new requests and stops
owned instances. Local file and command tools retain the level's policy.

## Levels

The working folder is the directory you started Jecode in, with all its
subdirectories.

| Level | In the working folder | Outside it |
| --- | --- | --- |
| **Read** | Read and search files; fetch web pages with GET or HEAD. No edits, commands or browser control. | Refused. |
| **Workspace** (default) | Read and edit files; run commands, [confined](#command-confinement) by the operating system. Network access is allowed. | Each file access, and each command started from an outside directory, asks for [approval](#approval-requests). |
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
| `permissions.default` in [`config.json`](../reference/CONFIGURATION.md#the-configuration-file) | The same default, edited by hand. |

The TUI shows the current level as a chip on the composer's lower border. A
level change applies to later tool calls. Calls already running, including
background processes, keep the authority they started with; stop a process from
`/process` if needed. Resume restores the conversation's level and approvals;
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
Ctrl+C stops the turn. Requests appear one at a time. A call too long for the
space shows `PgUp/PgDn` with the number of lines out of view: nothing is cut,
and the choices stay on screen while you read it.

Approving never switches the conversation to Full. To review approvals, open
`/permissions` → **Remembered** and press Ctrl+D on one to revoke it. Read and
Full pause remembered Workspace approvals without deleting them.

Collaborators share the conversation's level and approvals, and their requests
name the collaborator. In `--plain`, the main agent's requests appear as
numbered questions and closing stdin denies them; collaborators cannot ask
there. Single prompts and pipes cannot ask at all, so calls that need approval
are refused.

## Command confinement

Workspace runs `bash` and non-interactive `exec` commands inside an operating-system
confinement: a Process Security Environment on Windows, Landlock on Linux and
Seatbelt on macOS. The system enforces a filesystem policy on the command and every
process it starts:

- The working folder and the conversation's temporary area are writable. An `exec`
  call you approve with an outside working directory can also read and write that
  whole directory, for that call.
- System locations are readable, but your home folder, with SSH keys, cloud
  credentials and browser data, stays out of reach. On Linux and Windows nothing else
  is readable either. On macOS, external volumes and temporary folders are also closed,
  but other top-level folders, such as `/opt`, stay readable.
- Installed Rust toolchain data is readable, and Cargo's cache is redirected to
  the temporary area. Cargo's own configuration files (`.cargo/config.toml`) in the
  working folder's parents are readable too, because Cargo stops when it cannot open
  one; keep publishing tokens in `credentials.toml`, which stays unreadable.
  A linked ancestor configuration does not grant its target extra access; if
  that target is outside the permitted locations, choose Full.
  On macOS, Jecode locates the installed SDK before launching a Workspace command
  and supplies `SDKROOT` when unset, so Rust does not need `xcrun` to find it.
  An inherited or explicitly set `SDKROOT`, and an explicit removal, take precedence.
- Node.js runtimes installed through Volta, nvm, nvm-windows or fnm are readable,
  as is an install on `PATH` with npm beside it. On Linux and macOS, so are the
  folders on `PATH` inside your home, such as `~/.local/bin`, but not what their links
  point to: tools installed with pipx or uv need Full.
- On Linux and macOS, a program outside these locations cannot start, and the cache,
  settings and npm folders programs usually keep in your home (`XDG_CACHE_HOME`,
  `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_STATE_HOME`, `npm_config_cache`,
  `CLANG_MODULE_CACHE_PATH`) point at the temporary area. Your real ones are not used.
- `GIT_CONFIG_GLOBAL` points at the temporary area on every platform, so your global
  Git identity, aliases and credential helpers do not apply. Set `user.name` and
  `user.email` in the repository, or choose Full.
- On Windows, when `node.exe` is on `PATH`, including a version-manager shim, commands
  can also list the drives that hold the writable folders, the executable and
  configured manager homes, because Node.js checks each drive root while resolving
  script and module paths. Windows grants this listing for a whole drive: a command
  can see the names and metadata of files and folders there, including your user
  profile and the Jecode home, but cannot read their contents or write outside the
  writable folders. On Windows builds without listing-only support, Node.js scripts
  need Full. On Linux and macOS, names and metadata stay visible everywhere but
  listing a folder outside the grants does not work.
- The rest of the Jecode home, including configuration, credentials and other
  conversations, is outside the writable area.
- Commands inherit only a fixed set of operating-system and toolchain
  environment variables, not arbitrary tokens from your shell. Values set
  explicitly in an `exec` call still apply.
- On Linux and macOS, commands run without a controlling terminal, so they cannot
  type into Jecode or your shell and programs that ask for a password on `/dev/tty`
  fail at once.

If the Jecode home lies inside the working folder, Workspace refuses to run
commands. Start Jecode in a narrower project directory, or choose Full.
On Linux, a custom `JECODE_HOME` inside a system runtime folder such as `/opt`
also prevents Workspace command launch: Landlock cannot subtract that directory
from its system read grants. Move the home outside those folders, or choose Full.
On macOS, the policy explicitly protects the configured Jecode home wherever it
is stored, while allowing the current conversation's temporary area and the
owned shell executable.

## Platform limits

- **Systems that cannot confine commands refuse them.** Linux needs version 5.13 or
  later with Landlock enabled, and Windows needs Process Security Environment 1.1
  with filesystem and network policies, which older releases lack. There, every
  Workspace `bash` and `exec` call fails and says why. File tools still follow the
  Workspace rules. To let the agent run commands, choose Full for the conversation
  (`/permissions full`) or as the default. There is no automatic fallback to Full.
- **Older Linux kernels confine less.** Landlock does not cover `truncate` below
  Linux 6.2, or sending signals to your other programs below 6.12.
- **Local sockets on Linux and macOS.** A confined command can still connect to
  a Unix socket by its path. Variables that name one, like `SSH_AUTH_SOCK`, are not
  passed on, but the path can be guessed. A service reached this way acts outside the
  confinement: Docker can, and on Linux so can your session's D-Bus, through
  `systemd-run --user`. Do not rely on Workspace to contain a hostile command where
  these services run.
- **Terminal devices on Linux and macOS.** A confined command can still open your
  other terminal windows by device path and read or write them, though it cannot type
  into them as if you had.
- **macOS services.** Commands cannot send Apple Events or reach most system
  services, which `open`, `osascript`, `launchctl` and Xcode's `xcodebuild` need.
  Choose Full for those.
- **Project files above the working folder.** A tool that needs a parent folder's
  files, such as Cargo finding a workspace's `Cargo.toml`, gets a permission error.
  Start Jecode at the top of the project, or choose Full.
- **Tools that write to your home.** A program that insists on a location outside the
  redirected ones fails with a permission error; choose Full or set that program's
  own cache variable in the `exec` call.
- **Deep paths on Windows.** Working or session temporary paths can exceed the
  native adapter's legacy Windows path limits and prevent Workspace commands
  from starting. Use a shorter project or configuration location.
- **Interactive terminals** (`exec` with `tty: true`) require Full on every
  platform.
- **Browser control** requires Full, because browser file access and downloads
  are not confined. Closing or stopping a browser the conversation already
  started works at any level. In Read and Workspace, use the `web` tool for
  network retrieval.
- **Links on Windows.** A confined command cannot create junctions, even inside
  the working folder. Installs that link a folder, such as an npm `file:`
  dependency, fail with `EPERM`. Use npm's `--install-links` to copy instead, or
  choose Full.
- **Local servers on Windows.** A confined command can connect to a server on
  your machine, but host clients, including `web`, cannot connect to a server
  running inside the confinement. Start development servers in Full; you can
  return to Workspace afterwards and the server keeps running.
- **Windows services and CI workers.** In a service session (session 0), Windows
  cannot start some programs, including PowerShell, inside the confinement. Run
  such programs from an interactive user session, or choose Full. See the
  [Windows issue](https://github.com/microsoft/mxc/issues/1465).

## What permissions do not cover

Permissions do not isolate the whole Jecode process. File tools check paths
before they act, and confinement covers command processes; network
services you reach act with their own authority. Full is unconfined, and
confinement does not undo changes made inside the working folder. Use
your own version control, such as Git, to recover files after an unwanted
change.

Conversations save level changes, approvals, revocations and decisions. A
request that was pending when Jecode stopped is recorded as not executed on
resume; it is never replayed automatically.
