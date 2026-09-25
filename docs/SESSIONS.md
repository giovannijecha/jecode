# Session storage and recovery

Jecode keeps credentials and settings in `~/.jecode/v1/`. New conversations use
`sessions-v2/`, with one `<id>.log`, `<id>.head` and single-owner `<id>.lock`
per session. The earlier `sessions/<id>.json` format remains readable under its
original 16 MiB and 256-turn contract. Listing and resuming a v1 file do not
convert or rewrite it. `view_image` is available only for v2 sessions.

## Canonical log

The v2 log appends ordered begin, step, receipt, guidance and outcome revisions.
Every provider output item, call argument, receipt output, turn metric and user
message is canonical. An image receipt stores a SHA-256 reference, source path,
format, dimensions and byte count. Its binary payload is stored once under
`~/.jecode/v1/images/<session-id>/` and checked when projected; it is never
embedded in a canonical log event. A separate projection in the head holds the
lossy context summary, cursor and partial-compaction checkpoint. The controller can release
completed turns and steps from memory after a projection checkpoint; the log
keeps them. Listing reads bounded heads. Resume checks the committed log with a
fixed-size I/O buffer and rebuilds only the working suffix. Older turns can be
traversed in pages of at most 16 turns and 80 MiB of encoded events. If a single
turn exceeds that page budget, `Saved::canonical_turn_slices` returns its exact
ordered log-event JSON in resumable pages of at most 8 MiB and 64 slices. Each
slice identifies its event and byte range; callers reassemble an event before
decoding UTF-8 JSON. The cursor is tied to the committed head and rejects a
changed log snapshot. The terminal restores the working suffix and indicates
when earlier canonical history is on disk.

The head also caches recent prompts for composer recall. It retains at most 64,
then evicts the oldest as needed to fit the **encoded** 1 MiB head, including the
title, projection and integrity field. Recall entries keep their exact text and
order. Eviction changes only this cache; every submitted prompt remains in the
canonical log and can be read through older-history traversal. Existing v2 heads
use the same format and remain readable without conversion.

One log event is bounded by 80 MiB of encoded JSON, derived from the existing
bounded provider response and receipt sizes. The reader handles its bytes in
64 KiB I/O chunks; it never loads a complete session snapshot. A submitted
prompt remains limited to 8 KiB, streamed step text to 1 MiB, and one receipt
output to 1 MiB. These are per-item bounds, not session-age limits.

## Commit and recovery

A checkpoint appends changed canonical events, syncs the log, then atomically
replaces the small head with the committed byte offset and rolling checksum.
The head replacement is the logical commit boundary. A checkpoint failure stops
the worker before any subsequent effect that needed that record. If replacement
may have happened but its final sync reports an error, that owner refuses more
writes until the session is closed and its committed head is inspected on resume.
On resume, Jecode validates every frame and checksum inside the committed
boundary. A short or malformed tail *after* that boundary is uncommitted and
ignored; the next leased writer truncates it before appending. Damage inside
the committed prefix or a damaged head is reported as corruption, never as an
empty session. The rolling checksum detects accidental damage; it is not an
authentication mechanism against a writer with access to the user files.

A leased resume that finds an active last turn commits its interrupted outcome
before accepting a new turn. The exact recorded receipts remain in that turn;
no historical tool is executed. Compaction checkpoints commit absolute step and
guidance cursors together, including guidance at the next-step boundary.

The ordering of pre-effect unknown-outcome and post-effect exact-receipt
checkpoints remains unchanged. If execution was interrupted between them,
resume preserves an unknown outcome and never replays the historical tool.
Neither a checkpoint nor the log rolls back a shell command or file effect.
Cancellation joins the active worker before the owner exits.

`File::sync_all` and same-directory rename are the native persistence boundary.
On Linux Jecode also syncs the containing directory after creation and head
replacement. The current Windows native state layer syncs file contents but
does not provide a directory flush; a sudden power loss can therefore leave a
recent rename or new filename unavailable even after a successful call. Normal
process interruption and a torn trailing write recover at the committed head.
Filesystem, controller and hardware failures beyond those OS guarantees cannot
be promised away.

## Context slices and images

Generation requests are measured as encoded JSON, including full image Base64,
against the 8 MiB account HTTP body bound. Summary requests retain a 2 MiB
bound. A captured image stays in the model-facing request across failed or
cancelled generation, close/resume and explicit continuation. Sending the request
does not mark the pixels inspected: only a validated, completed response to an
image-bearing request ends that pending state. Compaction may summarize earlier
eligible context but does not summarize the pending image step. A text-only model
receives truthful references and cannot consume the pending visual state.
If the complete visual request exceeds the wire bound,
Jecode stops with an actionable error instead of omitting or truncating pixels.
Compact earlier work or create a smaller PNG, then use the saved `image_id` to
view it again. Once a validated visual response has completed, compaction
uses the textual image metadata and the model's visual findings. The summary
distinguishes those findings from the retained binary evidence; older images can
be inspected again with `view_image(image_id)` in the same session. A completed
step too large for one summary request is represented as ordered user-reference
slices. A large call and its
receipt can span slices; each slice names the same call and receipt association,
byte range and total. These are data items, never executable assistant calls or
tool-result protocol items. Jecode checkpoints a validated partial summary and
the next source byte after every successful slice. The main cursor advances only
after all required bytes are covered. Cancellation, invalid summary, transport
failure or checkpoint failure leaves the previous usable projection intact.
An unvalidated sent slice may be submitted again on explicit continuation to
rebuild the summary; that does not rerun a tool.

## Explicit v1 import

To continue a v1 session beyond its old storage bounds, run:

```text
jecode --workspace PATH import-session V1_SESSION_ID
```

`PATH` must identify its saved working directory. Jecode takes its lease, reads
the source under the v1 decoder, creates a new v2 ID, compares every decoded
canonical turn and projection after writing, checks that the source bytes did
not change, and only then marks the new ID verified. The original file remains
byte-for-byte intact. A failed or interrupted import may leave an unverified
v2 artifact; it is not listed or resumable. The command prints the new ID only
after verification. Use that ID with `jecode resume`.
