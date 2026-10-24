# Export a conversation

Type `/export` in the active conversation. Jecode writes a new
`JECODE-SESSION-<timestamp>-<process>-<counter>.json` in that conversation's
working directory; it never overwrites an existing file. `/new` starts a
separate export context. An export is a snapshot for inspection or sharing,
not a file that `jecode resume` can open. Use [saved sessions](SESSIONS.md) to
continue work.

The JSON uses `format: "jecode.conversation"` and `format_version: 1`. It
includes the Jecode version, export time, backend, model, effort, working directory,
conversation `messages` and local `events`. Messages retain the system prompt,
user messages, completed assistant content, tool calls and results. New tool calls
use Jecode's flat `id`, `name` and `arguments` fields. Native response metadata,
including reasoning fields, stays in `jecode_origin` with its backend and model;
legacy original messages retain their stored shape. Events retain local command results, compaction summaries,
recovery and turn errors, including incomplete streamed text. An export taken
during an operation can contain a tool call whose result has not arrived yet.
Unsent composer text and paused drafts are not included.

The additive `build` object records the executable's OS, architecture, profile
(`debug` or `release`, inferred from Rust debug assertions), and a fingerprint
of its bytes. `jecode --version` prints the same identity, which helps identify
an older installed executable with the same package version. The fingerprint
is diagnostic, not an authenticity check; it is `null` if the executable cannot
be read.

New `request_timing` events record each model attempt's backend, model,
operation, attempt number, start time, request byte count and outcome. Durations
are in milliseconds: `preparation_ms` ends at dispatch, `first_update_ms` runs
from dispatch to the first nonempty model text, reasoning or tool activity,
`response_ms` runs from dispatch to completion or failure, and `total_ms` covers
the attempt. A boundary not reached is `null`. Waiting includes upload and
remote queueing; these events do not separate those two costs. Ordinary request
preparation includes context projection and serialization. Initial model
metadata and compaction setup are outside that attempt; compaction model calls
have their own `context_compaction` timing events.

`tool_timing` events identify the call and record its start time,
`preparation_ms`, `execution_ms` and `total_ms`. Preparation includes checkpoint
setup; measurement ends when execution returns, before result persistence and
rendering. Parallel calls overlap, so their totals must not be added as elapsed
turn time. Both timing event types use Unix milliseconds for
`started_at_unix_ms`, monotonic clocks for durations, and `after_message` to
locate the attempt in the transcript. Timing events stay outside model history
and do not copy request payloads or credentials.

Credentials from registered backend accounts are masked, but other conversation
content is retained. Review the export before sharing it. Bash tool results
contain bounded output previews (16 KiB per stream by default) and references
to the complete streams in the personal session store; those files are not
embedded in the JSON. See
[TOOLS.md](TOOLS.md) for output references.

The export writes only this JSON file. References to attachments remain in
messages and annotations, but the referenced files are not copied into the
export. See [ATTACHMENTS.md](ATTACHMENTS.md#export).
