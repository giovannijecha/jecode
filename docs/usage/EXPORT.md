# Export a conversation

Type `/export` in the active conversation. Jecode writes a new
`JECODE-SESSION-<timestamp>-<process>-<counter>.json` in that conversation's
working directory; it never overwrites an existing file. `/new` starts a
separate export context. An export is a snapshot for inspection or sharing,
not a file that `jecode resume` can open. Use [saved sessions](SESSIONS.md) to
continue work.

The JSON uses `format: "jecode.conversation"` and `format_version: 1`. It
includes the Jecode version, export time, model, effort, working directory,
provider `messages` and local `events`. Messages retain the system prompt,
user messages, completed assistant content and reasoning fields, tool calls
and results. Events retain local command results, compaction summaries,
recovery and turn errors, including incomplete streamed text. An export taken
during an operation can contain a tool call whose result has not arrived yet.
Unsent composer text and paused drafts are not included.

The configured Jecode API key is masked, but other conversation content is
retained. Review the export before sharing it. Bash tool results contain the
last 64 KiB of each output stream and references to the complete streams in
the personal session store; those files are not embedded in the JSON. See
[TOOLS.md](TOOLS.md) for output references.

Attachments referenced by the conversation, including PDF annotation images,
are copied into a `.attachments` folder beside the JSON and listed with
relative paths in the top-level `attachments` array. If an attachment cannot
be copied, Jecode does not write the export. See
[ATTACHMENTS.md](ATTACHMENTS.md#export).
