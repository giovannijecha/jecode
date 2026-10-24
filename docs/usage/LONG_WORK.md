# Long-running work

A turn continues while the selected model requests tools. Jecode sets no
elapsed-time, model-request or tool-call count limit. A final response, user
interruption or an error needing intervention ends it. Jecode does not infer
completion from quiet output or repeated commands; inspect a running process
with [`process`](TOOLS.md#persistent-processes) when its state matters.

The activity row distinguishes preparing a request, waiting for the model,
thinking, writing a response, running tools and reconnecting. Its duration
belongs to the current phase; the finished-turn summary retains total elapsed
time. Preparation includes local context and attachment work. Waiting begins
at request dispatch, so it includes upload and remote service time. An export
records request sizes and timing boundaries for investigation; see
[EXPORT.md](EXPORT.md).

## Connection recovery

Temporary network failures, incomplete streaming connections, rate limits and
temporary provider errors retry the current model request until recovery or
interruption. Delays grow from 2 to 64 seconds and respect a longer provider
`Retry-After` value. The delay caps request frequency, not attempt count or
turn duration. The HTTP connection has a 10-second deadline, and 120 seconds
without response bytes triggers recovery; received chunks, including stream
comments, reset that idle timer.

An empty provider response is retried, but three empty responses for one
request return an error. Invalid requests, credential or credit failures, and
other permanent errors return control immediately. Interrupted or incomplete
responses never execute their partial tool calls. Completed tool results from
earlier requests remain in the conversation while a later request retries.

The TUI displays reconnection status and keeps input and interruption
available. Stop a TUI wait with Esc or Ctrl+C. Failed attempt text and
diagnostics remain in the saved session and export. If a tool command's
completion cannot be confirmed, its result is `unknown`; inspect its output
and workspace before retrying. The [tool guide](TOOLS.md#bash-execution-and-saved-output)
describes command output and process controls.

## Active context and original history

Jecode keeps original messages and tool results in the saved conversation and
export. After compaction, the next model request uses a smaller view built from
system instructions, a fixed excerpt of archived user requests, a summary and
recent original exchanges. The excerpt keeps the first request and the latest
requests that fit its budget; `history:requests` retains the complete sequence.
The context indicator in the TUI estimates the next request as a share of the
effective model window; `Context unknown` means the window has not been
established. It is an estimate, so a provider context rejection can still occur.

Input returned to the composer is withdrawn from the model's history view:
neither `history:N` nor `history:requests` exposes that attempt. Submitting it
again creates a new history entry. The original attempt remains in the saved
session and export for diagnosis; withdrawal does not erase archived data.

The [Auto-compact at setting](CONFIGURATION.md#automatic-compaction-threshold)
defaults to 80% of the effective window. Before each model request, reaching
that input budget first turns large older tool results into previews containing
`history_reference`. If that is insufficient, Jecode asks the same selected
model, with tools disabled, to fold the work since the previous compaction into
a plain-text handoff. It processes the transcript in chronological portions,
including large records split across portions. It carries the prior summary
and archived user requests into each portion. Recent original messages stay
verbatim when they fit within a quarter of the input budget. A tool call and
all its results stay together; an oversized group may leave no verbatim tail.
The saved originals are not replaced. Use `read(path="history:N")` or
`history:requests` to recover omitted content; [Tools](TOOLS.md#file-paths-and-paged-reads)
explains pagination and other references.

The handoff asks the model to retain goals, constraints, decisions, completed
work, errors and next steps. References to skills read before compaction are
saved alongside it so the model can reread them when relevant. Compaction stays
within the current turn, so it does not submit a draft or replay a completed
tool call. A provider context rejection can lower the effective window and
trigger reduction earlier than the configured percentage suggests. A truncated
ordinary response reports incomplete output without executing partial calls.

If the system instructions and original requests leave too little room for a
useful summary, or the new summary does not fit, the turn stops with a resource
error naming the budget. Raise **Auto-compact at** or select a model with more
context, then continue the retained conversation. An empty, interrupted or
unsaved summary also leaves the previous context and originals available.
Resume restores the saved context view; it does not restart work. See
[Sessions](SESSIONS.md) for resume and storage behavior.

Work runs only while Jecode is open. Disk space, memory, provider availability,
context capacity and account credit remain practical limits.
