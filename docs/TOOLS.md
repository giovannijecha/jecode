# Tools

Jecode exposes seven base tools when started with a workspace. It also exposes
`view_image` when the selected account model explicitly lists image input in its
catalog metadata and the session uses v2 storage. With no workspace, the model
receives no tools. Local UI commands and context management do not add tools.

| Tool | Purpose |
| --- | --- |
| `list_files` | Discover entries in a selected directory |
| `read_file` | Read a bounded range of UTF-8 text with line numbers |
| `recall_receipts` | Retrieve a saved workspace-read result from this session |
| `search_text` | Find literal text in supported local files |
| `create_file` | Create one new UTF-8 file directly |
| `edit_file` | Apply an exact text replacement directly |
| `run_command` | Run a non-interactive shell command directly |
| `view_image` | Submit a local PNG or saved image ID as visual input when available |

There is no dedicated web-search or browser tool. `run_command` can attempt network operations using
available local programs. Fetching a known URL, searching the web and interacting
with a browser require different methods; connectivity, installed programs and
remote services must be established from results. Command output is text. A
program can produce a screenshot file, then `view_image` can submit its pixels.
A screenshot path alone remains text, not visual input.

## Local image viewing

`view_image` accepts exactly one of `path` and `image_id`. `path` uses the selected
file-access profile and the same exclusions and no-link opens as other file tools.
It accepts ordinary PNG files up to 5 MiB. Jecode checks the PNG signature,
structure, dimensions, chunk checksums and zlib header; it does not decompress
pixels locally. A corrupt compressed stream that passes these container checks
may still be rejected by the provider. JPEG, GIF, WebP and animated PNG are not
supported. Jecode does not resize images. It sends the captured original PNG
bytes with a `high` detail hint; any provider-side image processing or effective
visual resolution has not been verified live.

The tool reports the source path, format, width, height, byte count and stable
`image_id`. The image is delivered as a multimodal content item paired with its
tool call. Raw bytes and Base64 are absent from tool receipts, diagnostics and
terminal output. Missing files, unsupported formats, invalid PNG containers,
read limits and storage failures have distinct errors. If a source is too large,
save a smaller PNG using an available program and view it again.

Each successful view saves the exact bytes under
`~/.jecode/v1/images/<session-id>/<sha256>.png` before committing its receipt.
The v2 log keeps only the digest and metadata; it does not copy the payload into
successive events. Changing or deleting the original path does not change what
resume sends. `view_image` with a prior `image_id` in the same session reopens
the saved copy, even after compaction. It checks the saved file's SHA-256,
length, PNG structure and dimensions. Missing or altered evidence blocks a
visual request with a recovery error; restore the private image store from a
backup. These digests detect accidental damage, not malicious rewriting of both
evidence and session records.

A saved view remains pending until a validated image-bearing response completes.
Failed, cancelled and incomplete requests do not clear it. Explicit continuation
resends the saved pixels without reopening the original file or replaying the
tool. Earlier context may be compacted, but the pending image stays in the
request. Switching to a text-only model shows text references; switching back
restores the pending pixels. Later validated visual responses allow ordinary
compaction of their earlier image receipts.

Jecode measures the full encoded request before accepting another view. If a
batch exceeds the 8 MiB account request budget, excess views receive a tool
error while accepted views remain paired and pending. Use a smaller PNG for the
rejected call. For an older session already blocked by saved pending pixels,
`/discard-pending-images` explicitly stops sending those pixels and records
that they were not inspected. The saved evidence and canonical receipts remain;
view an `image_id` again when it fits.

An interrupted or failed capture does not commit a successful view. An
incomplete temporary private write is not a valid image ID. A checkpoint failure
stops the worker; a saved payload without a committed receipt may remain as an
orphan and is never replayed as a tool. No automatic image cleanup runs. On
Windows, file contents are synced but the native state layer cannot guarantee
directory durability across sudden power loss.

Known files can be read directly. Directory exploration is for discovering unknown
paths. Tool results report omissions and truncation so a partial result is not
mistaken for a complete search. Tool output is bounded to 32 KiB of encoded JSON.
`read_file` scans an ordinary UTF-8 file with bounded memory and returns complete
lines in an 8 KiB page, at most 400 lines. It can read files larger than 1 MiB;
the scan checks the whole file and is subject to cancellation and the operation
time limit. `search_text` still scans only files up to 1 MiB within its separate
directory-wide budget.

## Recorded receipt recall

`recall_receipts` reads an original, committed `list_files`, `read_file` or
`search_text` result from the current session. It never reruns the historical
tool or opens the original source path. Supply the absolute canonical `turn`
and `step` from a compaction handoff, plus the zero-based `receipt` position in
that step's call order. The response includes the original call ID, tool name,
summary, exact output bytes as UTF-8 text, total byte count and a `next`
position. Follow `next` when it is present; its `offset` is a UTF-8 byte offset,
not a line number. The cursor skips effects and unexecuted or uncertain
receipts, while retaining the original receipt indices. A completed read in an
interrupted batch remains available even if a later sibling did not execute.
A response carries at most 8 KiB of original result text and
must also fit the 32 KiB encoded tool-output bound. Heavily escaped text may
therefore use smaller pages.

Absolute turn/step/receipt coordinates remain stable after compaction and
resume. The controller reads released v2 turns from the current session's
committed log in bounded turn pages; resident turns use the same coordinates.
Conversation-only sessions have no tool access. Command, edit, image and opaque
provider records are outside this retrieval tool. Each newly admitted recall
result remains in the next generation request until an accepted response
consumes it, even when the ordinary compaction threshold is exceeded. The
controller measures the encoded batch before admitting each recalled page and
reserves paired error results for remaining calls. When the batch reaches the
8 MiB request limit, it reports the unadmitted page and marks later calls as
not executed; request those pages again after consuming the delivered results.
For an older saved batch that already exceeds the limit, the next projection
delivers a fitting prefix and explicit paired deferred notices. The exact
canonical results remain saved; reissue the paired recall arguments to recover
deferred pages. If even the notices and other context cannot fit, the request
still fails with a history limit. Recall is evidence of an earlier observation;
use a separate deliberate read when the current state of a changed source
matters. A single old turn that
exceeds the existing 80 MiB turn-page budget cannot be recalled through this
tool, and a handoff may still omit or misstate a reference.

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
resource bounds. Ordinary summary requests retain a 2 MiB encoder bound; the
generation request uses an 8 MiB HTTP body bound to accommodate image Base64.
Its measured encoded bytes include the complete image item. New canonical sessions
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

One controller orders effects and records results. No failed model request is silently resent. Session
resume restores previous facts and waits for new input. An interrupted effect
without a durable receipt stays uncertain and requires inspection before repeating.
