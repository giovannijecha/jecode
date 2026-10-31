# Copy and export

`/copy` puts part of an answer on your clipboard. `/export` saves the whole
conversation as a JSON file. Neither makes a model request or adds anything to
the conversation.

## Copy

Type `/copy` to choose from the latest completed answer: the whole response, one
of its fenced code blocks or one of its quotes. It works while the agent is busy.
Use Up/Down and Enter; short menus also accept number keys and long ones let you
type to filter. Esc or Ctrl+C closes the menu without stopping work.

- **Whole response** copies the original Markdown.
- **Code** copies a block without its opening and closing fences, including code
  inside quotes.
- **Quote** copies the text without one outer `>` marker.

Spacing, Unicode and line endings are copied exactly, including trailing spaces
and newlines. Known credentials stay masked. The menu captures its source when
it opens, so a newer answer does not change your choice. Incomplete or partially
streamed answers are not offered; after `/new` there is nothing to copy until an
answer completes.

In `jecode --plain`, `/copy` shows a numbered menu. Enter picks the whole
response; `0`, `/cancel` or end of input closes it. Single prompts and pipes do
not run slash commands; redirect their stdout instead.

### Platform requirements

| Platform | How text is copied | Requirement |
| --- | --- | --- |
| Windows | `Set-Clipboard` through Windows PowerShell | Windows PowerShell and an available clipboard. |
| macOS | `/usr/bin/pbcopy` | The system pasteboard. |
| Linux | An OSC 52 request written to the terminal | A terminal that allows OSC 52 clipboard writes. |

Success shows `Copied … to clipboard`; failures are shown too. With OSC 52 the
terminal does not confirm, so Jecode shows
`Copy sent to terminal · confirmation unavailable`. Terminals may ignore the
request or ask for permission, and terminal multiplexers may need their own
clipboard setting. OSC 52 copies are limited to 100,000 bytes; larger selections
fail rather than being cut short.

On macOS, `pbcopy` would treat text starting with an EPS or RTF signature as rich
content, so such selections use OSC 52 instead when an interactive terminal is
available. Empty text and text containing NUL characters cannot be copied.

## Export

Type `/export` to write the current conversation to a new file in the working
directory, named `JECODE-SESSION-<timestamp>-<process>-<counter>.json`. Existing
files are never overwritten. An export is a snapshot for inspection or sharing;
`jecode resume` cannot open it.

The file is JSON with `"format": "jecode.conversation"` and
`"format_version": 1`. It contains:

- the Jecode version and a `build` object identifying the executable (OS,
  architecture, `debug` or `release`, and an FNV-1a fingerprint of its bytes,
  the same one `jecode --version` prints; the fingerprint is diagnostic, not an
  authenticity check);
- the export time, provider, model, effort and working directory;
- `messages`: the system prompt, your messages, completed answers, tool calls
  and results, with provider-specific data kept under `jecode_origin`;
- `events`: local command results, compaction summaries, recovery records, turn
  errors including incomplete streamed text, and timing records.

Unsent composer text is not included. An export taken while work is running can
contain a tool call whose result has not arrived yet.

### Timing records

`request_timing` events describe each model request attempt: provider, model,
operation (`completion`, `context_compaction` or `session_title`), effort,
attempt number, start time, request size, outcome (`complete`, `failed` or
`cancelled`) and the token counts the provider reported (`input_tokens`,
`output_tokens`, `total_tokens`, `cached_tokens`, `cache_write_tokens`,
`reasoning_tokens`; missing counters are `null`). Cached tokens are already
included in input and reasoning tokens in output, so do not add them again.
These counters do not tell you how a subscription weights its usage.

Durations are in milliseconds: `preparation_ms` (local work before the request
is sent), `notification_ms`, `first_update_ms` (until the first text, reasoning
or tool activity), `response_ms` and `total_ms`. Waiting time includes upload
and the provider's queue.

`tool_timing` events record each tool call's `preparation_ms`, `execution_ms`
and `total_ms`. Parallel calls overlap, so do not add their durations to get a
turn's elapsed time.

### Before sharing an export

Known credentials, including your OpenRouter key and account tokens, are masked.
Everything else is kept, including file contents the agent read and command
output previews. Review the file before sharing it. Complete command output and
attachments are referenced, not embedded, so they stay in your local storage.
