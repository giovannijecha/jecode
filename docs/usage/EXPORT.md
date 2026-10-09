# Export a conversation

Type `/export` to create a unique `JECODE-SESSION-<timestamp>-<process>-<counter>.json`
in the session's working directory. Existing files are never overwritten. Each
export is a snapshot of the current conversation, including the system prompt,
user messages, retained assistant content and reasoning fields, tool calls and
tool results. Streaming reconstructs the assistant message from ordered deltas.
Model/settings changes preserve this evidence; /new starts a new export context.
Conversations autosave from their first sent request; opening, drafting and local
commands alone do not create a saved session. Use `/resume` or
`jecode resume` from the same project folder to continue one. See [SESSIONS.md](SESSIONS.md)
for recovery and deletion behavior; exports retain their separate snapshot format.
In the `/resume` list, Ctrl+D marks the selected row for deletion, Enter confirms
and Esc cancels. The current conversation is included after its first sent request;
deleting it keeps unsent
draft/queue input and starts a fresh context while the list stays open. Empty
launches and unsent input alone never add a current row. Use Esc to return to
the composer, including after deleting the last session. Project files and manual exports remain.

## Format and retained evidence

The JSON contains `format: "jecode.conversation"`, `format_version: 1`, the Jecode
version, export time in Unix milliseconds, model, effort, working directory,
`messages` and `events`. Events record local command results, compaction summaries,
request recovery and turn errors,
including partial streamed text, separately from provider messages.
Tool results remain their original JSON-encoded `content` strings, paired with
`tool_call_id`, matching the provider conversation. Local commands, screen
formatting and UI notifications are not model messages. An export taken during
an operation can include a call whose result has not arrived yet.

The configured Jecode API key is masked in the export; other session content is
retained. Bash results include the last 64 KiB of each stream and references to
complete output saved separately in the folder's personal session bucket. These
files are not embedded in the export. File reads include their range and
continuation offsets. Follow those indicators when interpreting an exported
session.
