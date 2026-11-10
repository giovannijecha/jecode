# Long-running work

A turn keeps going for as long as the model keeps calling tools. Jecode sets no
limit on elapsed time, model requests or tool calls, and it does not guess that
work is finished from quiet output or repeated commands. A turn ends when the
model answers without a tool call, when you stop it (Esc or Ctrl+C in the TUI),
or on an error that needs you. Work runs only while Jecode is open.

## Connection problems

Temporary network failures, dropped streams, rate limits and temporary provider
errors retry the current request until it succeeds or you stop it. The delay
between attempts grows from 2 to 64 seconds and honors a longer `Retry-After`
from the provider; there is no attempt limit. The TUI status row shows the
countdown, attempt number and reason, and you can keep typing or stop the turn.

- A request that receives no data for 120 seconds is retried. Connecting has a
  10-second deadline; on Linux and macOS it bounds each address attempt.
- An empty response is retried; three empty responses for the same request end
  the turn with an error.
- Invalid requests, rejected credentials, invalid local proxy settings, TLS
  certificate failures, a ChatGPT plan's usage limit, insufficient OpenRouter
  credit and other permanent errors stop immediately.
- An interrupted or incomplete response never runs its partial tool calls.
  Results of tool calls from earlier requests stay in the conversation.

Failed attempts and their diagnostics stay in the saved conversation and in
[exports](COPY_AND_EXPORT.md#export). If a command's completion cannot be
confirmed, its result is `unknown`; check its output and the project before
retrying.

## Context and compaction

Every model has a context window. The TUI footer shows how much of it the latest
request used, as a share of the window, filling toward the **Auto-compact at**
threshold (default 80%). A leading `~` means the figure is Jecode's estimate of
the next request, shown when it differs from the last provider reading by at
least one point of the window. `?` means no usage has been reported yet or the
window is unknown. The figure includes cached input. It is not your plan's
usage or remaining quota. The `↑` and `↓` counts in the status row are the
turn's input tokens sent and generated output, not context.

Until the request reaches that threshold, Jecode only adds to it: earlier
messages are sent again unchanged, so the provider can serve them from its
prompt cache.

When a request would exceed the threshold, the same model summarizes the work
since the last compaction into a plain-text handoff of at most about 16 KiB. It
first asks over the conversation's own request, with its tools and your selected
effort, allowing the provider to reuse a cached prefix. If that does not fit or
fails, it summarizes the work in portions at medium effort when the model
supports it, with tools disabled. The summary keeps goals, constraints,
decisions, completed work, errors and next steps. Your model and effort for
the task do not change.
Your context meter drops when this happens, and a notice says how many messages
were summarized.

The next request then contains the instructions, an excerpt of your earlier
requests (the first one and the latest that fit), the summary and the most
recent messages verbatim. The recent part uses up to a quarter of the budget,
and a tool call always stays together with its results. The provider caches the
new request from there.

Compaction never deletes anything. The full original history stays in the saved
conversation and export, and the model can read any part of it again with
`history:N` or the complete list of your requests with `history:requests`; see
the [Tool reference](TOOL_REFERENCE.md#read). Skills the model had read are
remembered so it can read them again. Compaction happens inside the current
turn: it does not send your draft or rerun a completed tool call.

[Saved conversations](SESSIONS.md#after-an-interruption) must use the current
storage format. Experimental versions do not migrate older formats, and leave
unsupported files unchanged.

If a provider rejects a request as too large, Jecode lowers its estimate of the
window and compacts earlier. If even the instructions, your requests and a
minimal summary do not fit, the turn stops with an error naming the budget:
raise **Auto-compact at** or choose a model with a larger window, then continue.
A failed or interrupted summary leaves the previous context in place.

The context window comes from the provider's catalog. The OpenAI API model list
reports none, so Jecode uses the documented window of recognized API models;
with other API models the window stays unknown, and Jecode reduces the context
only after the provider rejects a request as too long. Your account may offer
less than a model's public specification.

## Input you take back

When a turn fails or stops before producing any answer or tool result, your
message returns to the composer. Taken-back input is removed from what the model
sees, including `history:N`, but stays in the saved conversation and export for
diagnosis. Sending it again creates a new entry.

## Practical limits

Disk space, memory, provider availability, the context window and your plan's
usage or credit still bound long work.
