# Permissions

Each conversation has a user-selected permission level. Jecode applies its
boundaries; the model does not classify danger or grant itself more authority.

| Level | Inside the working folder and its subfolders | Outside the working folder |
| --- | --- | --- |
| **Read** | Read and analyze. Project edits, commands and checkpoint restores are unavailable. | Unavailable. |
| **Workspace** (default) | Edit files and run native commands within the recursive workspace and authorized session scratch. Network access is enabled. | File access and an external command working directory ask first. |
| **Full** | Read, write and execute with the invoking user's OS permissions. | The same authority applies here. |

Read still permits Jecode to save conversation state and use its own temporary
storage. It never asks for project writes: select another level instead.
Workspace and Full can reach the network; the level is not a domain allowlist.

## Choosing a level

| Action | Scope |
| --- | --- |
| TUI `/permissions` | Choose this conversation's level, or open **Remembered**. |
| `/permissions read`, `workspace` or `full` | Change this conversation's level. |
| `/settings` → **Default permissions** | Save the level for new conversations. |
| `--plain`: `/permissions` | Print the levels, current selection and saved default. |
| `--plain`: `/permissions remembered` | List remembered grants. |
| `--plain`: `/permissions revoke N` | Revoke the numbered grant. |

Changing a saved default leaves the current conversation unchanged. `/new`
uses that default; resume restores the conversation's level and grants.
The TUI names the current level on the composer's bottom rule.

Level changes apply to subsequent calls. Already running calls and background
processes retain their starting authority. The notice identifies this; use
`/terminal` to stop a process. Revoking a grant likewise affects later calls.

## Windows commands

Workspace runs the owned `bash` interpreter and non-interactive `exec` inside a native Windows Process
Security Environment. The OS enforces the filesystem policy on the command
and its descendants. The workspace grant includes all subfolders. Session
scratch is writable; installed Rust runtime data is read-only when present,
and Cargo's default cache is redirected to session scratch. Explicit command
environment overrides do not widen the OS filesystem policy.

Confined commands inherit a fixed set of OS and toolchain environment variables,
not the invoking shell's arbitrary tokens and secrets. Explicit `exec.env`
values and removals still apply. Full retains normal environment inheritance.

Workspace does not grant access to all of `~/.jecode`. Configuration,
credentials and unrelated sessions are outside the command's writable roots.
If Jecode's configuration directory lies within a writable root, launch is
refused: choose a narrower project directory or select Full explicitly.

The `bash` tool uses Jecode's embedded interpreter and needs no Git Bash or
installed Bash. Its exact executable has read-only access inside the native
boundary; this grants no access to adjacent credentials or configuration files.
Use `exec` for explicit native programs. Interactive `exec`
(`tty:true`) also requires Full: on the verified Windows build, combining
ConPTY with the native security environment loses terminal output. Workspace
rejects that combination before launching a process.

On the verified Windows 26H2 build, isolated commands can connect to a host
localhost service, but the host cannot connect to a server inside the isolated
environment. **Start local development servers explicitly in Full.** You can
then return to Workspace; the server keeps its starting Full authority.
Jecode does not change global firewall rules or loopback exemptions.

Confined commands require native Process Security Environment 1.1 with the
required filesystem and network policy support. Unsupported Windows hosts,
Linux and macOS reject Workspace command launch; select Full explicitly to run
unconfined commands there. There is no automatic fallback to Full.

On the verified Windows host, services and CI workers in session 0 cannot start
some programs, including PowerShell, inside PSEC: Windows fails their DLL
initialization before user code runs. The owned interpreter and native commands
without that requirement remain confined and usable. Run affected programs in
an interactive user session or choose Full explicitly. See the
[Windows PSEC issue](https://github.com/microsoft/mxc/issues/1465).

## Requests

Workspace waits before a file call outside its folder or an `exec` with an
outside working directory. The request identifies the caller, target and
access before anything runs:

- **Approve** runs this call once.
- **Approve and remember** grants the same access for this conversation,
  including after resume. File and folder grants match their exact target;
  command grants also distinguish arguments, explicit environment, input and
  terminal parameters. No command prefix is granted.
- **Deny** runs nothing and returns the decision to the agent.

An exception does not switch the conversation to Full. In the TUI, typed
1–3 choose; pasted text never approves. Up/down then Enter also chooses,
while Enter without a focused option does nothing. Ctrl+C stops the turn.
Requests are shown one at a time and keep the conversation draft. A request
that arrives while you are typing decides nothing: further keys keep editing
the draft until they pause briefly or an arrow or Tab moves into the request.

The line-based `--plain` chat asks numbered questions for the main agent;
closed input denies. Collaborators cannot ask there. Single prompts and pipes
cannot ask, so calls needing approval are unavailable in those modes.

`/permissions` → **Remembered** lists grants; Ctrl+D revokes one. Read and
Full pause remembered Workspace exceptions without deleting them.
Collaborators share the level and grants, and their TUI requests name the
asking agent. An isolated collaborator's owned project copy is its workspace.

## Other tools and limits

Read allows web GET/HEAD retrieval without a request body, but not explicit
modifying HTTP requests. Browser control currently requires Full because its
local-file access and download destinations are not confined by the command
policy. Closing or stopping an already owned browser remains available.
Use `web` for network retrieval in Read or Workspace.

These permissions do not isolate the entire Jecode process. Native file tools
resolve and check paths; Windows command isolation covers command processes
and descendants. Network access can reach services with their own authority.
Full is unconfined, and filesystem confinement does not undo allowed project
changes. Checkpoints provide a separate recovery mechanism.

## Stored settings and history

The default is `permissions.default` in `~/.jecode/config.json`: `read`,
`workspace` or `full`. A missing value means `workspace`. Conversations save
level changes, grants, revocations and decisions. An unresolved request is
recorded as not executed on resume, rather than automatically replayed.
