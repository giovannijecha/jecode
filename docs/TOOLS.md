# Tools

Jecode exposes six tools when started with an explicit workspace. With no workspace,
the model receives none. Local UI commands and context management do not add tools
to the model schema.

| Tool | Purpose |
| --- | --- |
| `list_files` | Discover entries in a selected directory |
| `read_file` | Read a bounded range of UTF-8 text with line numbers |
| `search_text` | Find literal text in supported local files |
| `create_file` | Create one new UTF-8 file directly |
| `edit_file` | Apply an exact text replacement directly |
| `run_command` | Run a non-interactive shell command directly |

Known files can be read directly. Directory exploration is for discovering unknown
paths. Tool results report omissions and truncation so a partial result is not
mistaken for a complete search. Tool output is bounded to 32 KiB of encoded JSON.
`read_file` scans an ordinary UTF-8 file with bounded memory and returns complete
lines in an 8 KiB page, at most 400 lines. It can read files larger than 1 MiB;
the scan checks the whole file and is subject to cancellation and the operation
time limit. `search_text` still scans only files up to 1 MiB within its separate
directory-wide budget.

## Working directory and access

The default `local` profile accepts paths relative to the working directory,
including `../sibling/file`, and native absolute paths. Windows accepts drive paths
with either slash style. Changing a command's starting directory does not change
the base for later file tools. The model receives that base and the active profile.

The `workspace` profile retains the narrower contract: forward-slash relative
paths inside the selected directory, without parent traversal. Choose it with
`--access workspace` or the `file_access` user setting. Resume uses the session's
saved profile, not a newly changed default. Sessions saved before access profiles
were added retain `workspace` access.

Both profiles reject links, reparse points, network/device paths, dot paths and
known credential names. These name checks cannot identify secrets in arbitrary
file contents. In `local`, generated directories are omitted from discovery but
known files inside them can be addressed directly. Paths are normalized before
opening through native directory handles. External changes and commands show
their absolute destination in the activity display. Valid model calls execute
directly under the selected file-access profile.

Treat file content and command output as potentially sensitive: requested contents
are sent to the provider. Do not put secrets in a task that asks the model to print
or summarize them.

## File changes

Jecode prepares the exact change before execution. Creation does not overwrite an
existing file. Editing requires one exact occurrence of the old text and rejects
changes made after the preview. The model is instructed to read before editing.
Large existing files use a temporary, owned snapshot so preparation and stale
checks do not keep multiple full copies in memory. Existing originals have an
adjacent `.jecode-recovery-*` copy after publication, referenced in the receipt.
Do not remove recovery copies until they are no longer needed.

The visible diff shows at most 48 KiB and 400 lines. When shortened, it reports
how many rendered lines and bytes were omitted. Display limits do not shorten the
change itself.
For a large existing file, the diff shows the exact replacement
and byte offset while omitting unchanged surrounding file context. Temporary
`.jecode-snapshot-*` files are also removed on normal completion or cancellation.
An abrupt process exit can leave a temporary file behind; it does not replace
the target or serve as a recovery copy.

The old 32 KiB proposal and 1 MiB edit-file caps are gone. Account responses
still have a 1 MiB decoded event and output budget, so the encoded tool call,
including JSON escaping and other response items, must fit those protocol
resource bounds. Request context has a 2 MiB budget; new canonical sessions
append bounded log records instead of one whole-session snapshot. A response
that exceeds the provider budget is rejected before any tool runs. Jecode saves
an uncertain pre-effect receipt and then the exact result. A required checkpoint
failure stops later effects. An interrupted effect without a durable exact receipt
stays uncertain and requires workspace inspection before repeating it. Historical
denial receipts remain history; resume never schedules them as new work.

## Commands

Windows uses the system Windows PowerShell 5.1 by default, or an explicitly
configured PowerShell 7 executable. Linux uses `/bin/sh`. The activity display
shows the script, starting directory and timeout. Scripts are limited to 4,096
UTF-8 bytes; timeouts are between 1 and 300 seconds. Output streams through bounded
pipes, with an explicit exit code, truncation and cleanup status in the receipt.
If the terminal falls behind, live output chunks can be omitted without pausing
process supervision; the receipt marks truncation and retains bounded stream tails.
PowerShell parser and runtime errors appear as readable stderr with a failing exit
code. Windows command starting directories must fit the Win32 `MAX_PATH` current
directory limit; a longer directory fails before the script runs, even if file tools
can access it. Windows PowerShell 5.1 cannot safely resolve relative literal
paths from a starting directory containing `[` or `]`; Jecode rejects that
command before launch and explains how to configure PowerShell 7. PowerShell
7.6.6 is the version exercised in CI with physical working directories, parent
traversal, relative cmdlets and native child processes. For any explicitly
configured PowerShell 7, Jecode checks bracketed-directory behavior once when
resolving the session shell, using an isolated temporary fixture. A successful
check permits bracketed command starting directories. An incompatible or
inconclusive check blocks commands starting there and reports the reason; ordinary
starting directories remain available. No other PowerShell 7 release is claimed
to be verified by CI. See [usage](USAGE.md) for the user-scoped shell setting.

This is not an interactive PTY and not a sandbox. The shell has your user
permissions, including access outside the selected starting directory. Process
cleanup uses a Windows Job Object or a Linux process group. Esc cancels and waits
for cleanup, but cannot reverse effects already performed.

## Ordering and recovery

One controller orders effects and records results. A turn is bounded to eight
model requests and 32 tools. No failed model request is silently resent. Session
resume restores previous facts and waits for new input. An interrupted effect
without a durable receipt stays uncertain and requires inspection before repeating.
