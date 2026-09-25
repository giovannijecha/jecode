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

There is no dedicated web-search or image-viewing tool, and Jecode does not send
image inputs to the model. `run_command` can attempt network operations using
available local programs. Fetching a known URL, searching the web and interacting
with a browser require different methods; connectivity, installed programs and
remote services must be established from results. Command output is text, so a
produced screenshot is not visual input to the model. Browser automation and
native web search are separate planned capabilities.

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
checks do not keep multiple full copies in memory. Before publishing an edit,
Jecode streams the exact original and proposed result into private files under
`~/.jecode/v1/recoveries/`, syncs them and records their workspace, absolute
target, session and tool call. The manifest also records SHA-256 digests computed
from both source streams before publication. Restore and repair recheck those
digests in bounded chunks before using either retained copy or removing an
adjacent original. The receipt's `recovery` value is a stable ID,
not a project path. Each edit retains two full versions; Jecode does not purge
them by age or count. The user owns this state and decides when it is no longer
needed. Creation has no previous version to retain.

The visible diff shows at most 48 KiB and 400 lines. When shortened, it reports
how many rendered lines and bytes were omitted. Display limits do not shorten the
change itself.
For a large existing file, the diff shows the exact replacement
and byte offset while omitting unchanged surrounding file context. Temporary
`.jecode-snapshot-*` and `.jecode-staging-*` files are removed on normal
completion or cancellation. Publication briefly uses an adjacent
`.jecode-recovery-*` name for the original on the target filesystem. It is
removed after a successful edit; the durable original is already in private
state. A crash or cleanup failure can leave an adjacent transient. Inspect it
and the recovery ID before taking action; Jecode does not delete old or
interrupted files automatically. User state and the project may be on different
filesystems: Jecode copies to private state in 16 KiB chunks and uses only
same-directory, no-overwrite renames for publication.
Linux flushes the parent directory after publication and cleanup. On Windows,
the file data and private manifest are flushed, but this implementation has
no directory-flush guarantee across sudden power loss; use `show` and `repair`
if the observed name differs from the recorded state after a crash.

## Inspecting and restoring files

Run these commands with the same selected working directory as the originating
session. They work after closing Jecode and do not contact the model:

```text
jecode --workspace PATH recover list
jecode --workspace PATH recover show ID
jecode --workspace PATH recover cat ID
jecode --workspace PATH recover restore ID
jecode --workspace PATH recover repair ID
```

`list` identifies versions by ID, target, session and operation. `show` reports
the recorded state, sizes, integrity status of both private copies, whether the
current target matches a verified version, and any adjacent transient.
`cat` streams the retained original to stdout for inspection or redirection;
it can contain sensitive project content. It warns when integrity is unverified;
its output is then for manual inspection only. `restore` installs that original only
when the current target is the exact recorded result with its expected file
identity, metadata and bytes, and both private copies pass their capture-time
integrity checks. A conflict or failed integrity check leaves the target
untouched. To walk
back several Jecode edits, restore the newest version first, then the preceding
one. Restoration preserves the original file's ordinary Unix mode or Windows
DACL access entries and protection mode, and its modification time. Windows
may normalize the DACL auto-inherited marker. Restoration is itself a visible
filesystem effect and may leave an incomplete record if interrupted. `repair ID`
reconciles a captured edit or interrupted restoration after inspecting the
target and adjacent file. It also removes a verified adjacent transient left
by an applied edit. Recorded file identities help distinguish Jecode's files
from competing files with identical contents. Repair refuses conflicts and
never replaces an occupied name. An interrupted repair records its intended
stage name and identity before publication; a retry verifies and removes its
own unpublished stage, or confirms its published result and finishes cleanup.
A recorded restored result with a remaining adjacent
transient can also be reconciled without publishing again.

The private state records `capturing` before copying either private file,
then `captured` before an edit can move the target,
`applied` after publication, and `restoring` before restoration. If the final
record fails after publication, the operation reports the applied result and
stops later effects; `repair` can confirm the result from the retained bytes.
If publication fails after moving the original aside, Jecode tries a
no-overwrite move back. A competing target wins; the original remains adjacent
and in private state. An interrupted transaction may need `show` and `repair`
before restoration. No recovery copy rolls back arbitrary shell effects.
The digests detect changes to the retained content; they are stored beside the
copies and do not authenticate against someone who can rewrite both copies and
their manifest.

Recovery manifests written before capture-time digests were added remain
inspectable. `show` labels them `unverified legacy`, and `cat` can stream their
original for manual inspection with a warning. Automatic `restore` and `repair`
refuse them, including a same-length file that appears unchanged. Jecode does
not hash their current contents and treat that as evidence of what was captured.
Leave these records and any adjacent original in place until you have compared
them manually and resolved any conflict.

Older receipts that contain an adjacent `.jecode-recovery-*` path remain valid
historical references. Those files are user data and are never imported,
moved or removed automatically. To use one, inspect both it and the current
target, make a separate copy, and restore it manually only after resolving
any target conflict. The new ID commands apply only to versions in private
state; they do not reinterpret historical paths.

There is no automatic or in-app cleanup of retained versions. To discard a
completed version deliberately, close the originating session, inspect its ID,
and remove exactly its `ID.json`, `ID.before` and `ID.after` files from the
private `recoveries` directory after making any desired backup. Preserve
`capturing`, `captured` or `restoring` records and any adjacent transient until their
outcome has been reconciled. Deleting a version makes its receipt a historical
reference only; Jecode will not silently rebuild it from the project.

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
