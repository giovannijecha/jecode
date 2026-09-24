# Tools

Jecode exposes six tools when started with an explicit workspace. With no workspace,
the model receives none. Local UI commands and context management do not add tools
to the model schema.

| Tool | Purpose | Approval |
| --- | --- | --- |
| `list_files` | Discover entries in a selected directory | No |
| `read_file` | Read a bounded range of UTF-8 text with line numbers | No |
| `search_text` | Find literal text in supported local files | No |
| `create_file` | Propose one new UTF-8 file | Once per proposal |
| `edit_file` | Propose an exact text replacement in an existing file | Once per proposal |
| `run_command` | Run a non-interactive shell command | Once per proposal |

Known files can be read directly. Directory exploration is for discovering unknown
paths. Tool results report omissions and truncation so a partial result is not
mistaken for a complete search. Tool output is bounded to 32 KiB of encoded JSON.
Reads operate on supported ordinary text files up to 1 MiB.

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
their absolute destination before approval. A path alone never authorizes an effect.

Treat file content and command output as potentially sensitive: requested contents
are sent to the provider. Do not put secrets in a task that asks the model to print
or summarize them.

## File changes

Jecode prepares the complete change and displays its diff before approval. Creation
does not overwrite an existing file. Editing requires one exact occurrence of the
old text and rejects changes made after the preview. The model is instructed to
read before editing. The proposal text budget is 32 KiB; resulting files stay
within the 1 MiB limit. Existing originals have an adjacent `.jecode-recovery-*`
copy, referenced in the receipt. Do not remove recovery copies until they are no
longer needed.

Approval is tied to the exact proposal. Denial disables further effects for that
turn; reads may still complete. Cancellation does not grant approval.

## Commands

Windows uses the system Windows PowerShell 5.1 by default, or an explicitly
configured PowerShell 7 executable. Linux uses `/bin/sh`. The proposal
shows the script, starting directory and timeout. Scripts are limited to 4,096
UTF-8 bytes; timeouts are between 1 and 300 seconds. Output streams through bounded
pipes, with an explicit exit code, truncation and cleanup status in the receipt.
PowerShell parser and runtime errors appear as readable stderr with a failing exit
code. Windows command starting directories must fit the Win32 `MAX_PATH` current
directory limit; a longer directory fails before the script runs, even if file tools
can access it. Windows PowerShell 5.1 cannot safely resolve relative literal
paths from a starting directory containing `[` or `]`; Jecode rejects that
command before approval and explains how to configure PowerShell 7. PowerShell
7.6.6 has been verified with physical working directories, parent traversal,
relative cmdlets and native child processes. Other PowerShell 7 versions can be
selected for ordinary directories, but bracketed starting directories are
rejected until that version is verified. See [usage](USAGE.md) for the
user-scoped shell setting.

This is not an interactive PTY and not a sandbox. The approved shell has your user
permissions, including access outside the selected starting directory. Process
cleanup uses a Windows Job Object or a Linux process group. Esc cancels and waits
for cleanup, but cannot reverse effects already performed.

## Ordering and recovery

One controller orders effects and records results. A turn is bounded to eight
model requests and 32 tools. No failed model request is silently resent. Session
resume restores previous facts and waits for new input. An interrupted effect
without a durable receipt stays uncertain and requires inspection before repeating.
