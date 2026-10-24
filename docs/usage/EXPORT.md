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

Credentials from registered backend accounts are masked, but other conversation
content is retained. Review the export before sharing it. Bash tool results
contain bounded output previews (16 KiB per stream by default) and references
to the complete streams in the personal session store; those files are not
embedded in the JSON. See
[TOOLS.md](TOOLS.md) for output references.

The export writes only this JSON file. References to attachments remain in
messages and annotations, but the referenced files are not copied into the
export. See [ATTACHMENTS.md](ATTACHMENTS.md#export).
