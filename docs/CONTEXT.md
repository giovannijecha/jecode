# Context management

Jecode keeps the canonical conversation separate from the input sent to the
model. Context management may summarize an older prefix or temporarily shorten
tool results to fit a request. It never deletes saved messages, changes tool
outcomes, replays historical tools, or edits the transcript and export.

## Request lifecycle

Before each model request, including tool follow-ups, the controller:

1. Resolves the selected model's usable capacity through its provider adapter.
2. Measures the complete outgoing input: instructions, conversation, tools,
   and the provider's relevant protocol content.
3. Asks the context manager to compact when measured pressure reaches the
   configured threshold. A tool-result character count cannot trigger this.
4. Measures any replacement context again. If it still cannot fit, applies a
   bounded emergency projection to tool results and measures that projection.
5. Reserves a useful response budget, clamps the configured output ceiling,
   and sends the streaming request. An irreducible oversized input fails locally.
6. Associates returned usage with the exact input that produced it.

Completed answers and settled tool batches are checkpointed without starting
another compaction. Automatic compaction runs only when another model request
is needed; a completed answer therefore hands control back without a trailing
summary request. `/compact` remains an explicit operation on the active leaf.

## Token measurement

There is no claim of a universal exact offline tokenizer. Tokenization depends
on the model, and protocol framing and opaque reasoning also affect context.
Jecode's installed runtime has no tokenizer dependency, downloaded vocabulary,
or extra counting request on each tool boundary.

`Provider.measureInput` is a local adapter boundary. It measures the same wire
conversion used for generation, counting the outgoing representation once.
Unsent normalized duplicates, foreign raw blocks, and accounting metadata do
not count as prompt text. Providers without this hook use a conservative
normalized-content estimate.

The owned estimator combines UTF-8 byte, compression, and literal-content
floors. It yields between bounded chunks and supports cancellation. Its values
are estimates, not billing counts. Known opaque reasoning fields are not
treated as tokenizable ciphertext when the same assistant response has valid
output usage: the adapter reserves the greater of the visible estimate and
reported output tokens once for that message. Without valid usage, opaque data
retains a conservative byte estimate. Raw protocol data itself is never changed.

Across consecutive turns of an open conversation, the input meter anchors a
successfully sent prefix to the provider's reported input count and adds the conservative estimate of newly
appended content. The observation is valid only while instructions, tools,
model, effort, and the entire measured prefix remain unchanged. Changed raw
data, compaction, emergency projection, or a replacement history invalidates it.
Only hashes and numeric measurements are retained by the meter.

The observation belongs to the open conversation, not to a single turn. A new
message therefore does not discard valid provider calibration and revert to
the larger local estimate. `/new`, timeline selection, accepted manual compaction,
provider access interactions, and failed or interrupted turns reset its lifetime.
Provider routes remain separate. Reset generations also reject a late observation
from a request that started before the reset. Restart/resume begins without an
observation: persisted usage alone cannot prove that the wire prefix is unchanged.

The most recent historical usage value is not an estimate of a newly extended
or resumed conversation. Cumulative usage is accounting, not context pressure.
Missing or malformed provider counts never create a measured baseline.

## Capacity and budgets

Provider metadata determines the usable window when available. A stricter
provider compaction ceiling and the request safety reserve still win over the
saved percentage. Unknown capacity uses the conservative 200,000-token fallback;
discovery never silently upgrades a default window to a larger advertised maximum.

`/settings` retains its 85% default and 50%-95% range. The ordinary target is
one quarter of the usable window; the recent-tail budget is half that target.
Planning uses provider-aware message estimates, including opaque replay
reserves, and safe boundaries so a retained tool call stays with its results.
Manual and overflow requests bypass the trigger and may reduce the retention
target to half the current input size, making explicit compaction useful below
the ordinary threshold. An individually impossible recent exchange may need
to be summarized as well.

Ordinary requests keep full tool outputs within the tools' own collection
limits. There is no separate 16 KiB per-result request excerpt. Existing input
therefore stays stable as tools append. Only an input that cannot leave the
minimum response budget uses the emergency projection. That projection favors
recent evidence, preserves call IDs and error state, marks omissions, and is
measured again, including Unicode and protocol overhead. Its character
allocation is a starting bound, never a token count.

## Summary workflow and recovery

The single controller makes one streamed, tool-free summary request using the
selected provider and model. It uses `low` effort when the model advertises it;
otherwise it keeps the selected effort. Source content remains untrusted
historical data, and the resulting memory is a user-level context message,
never a new system instruction.

The summary request has a 60-second deadline and observes turn cancellation.
The usual local output ceiling is at most 4,096 tokens. Provider transport
limitations still apply: OpenAI Account does not send that ceiling to the
server. Returned summaries must be nonempty and at most 32,768 code units.
Acceptance additionally requires at least 20% estimated input savings, at
least 256 tokens saved, and a result below both the automatic trigger and the
safe request limit. A weak or oversized result leaves the previous context intact.

Automatic failures are suppressed until input grows meaningfully; an extra
message alone is insufficient. A definite provider context-limit rejection
has one separate recovery opportunity. New conversations, manual compaction,
and timeline selection reset the automatic breaker. Changed model capacity
invalidates its pressure scope. Ambiguous generation failures are never retried
as context recovery.

Usage from a returned summary is accounted even when its text is rejected.
The owned `jecode.context` diagnostics channel emits numeric request measurements
and compaction outcomes, including failures and cancellation. No listener is
installed in the ordinary product, and nothing is written to session files or
exports. The [development recorder](../dev/context/README.md) can capture bounded,
content-free evidence during real work. An `accepted` event means the summary
passed its checks; durable commit still follows the checkpoint contract below.
The footer continues to show the `Compacting` phase with interruption available.

Accepted memory uses the existing branch-local summary anchor and is committed
with the next valid turn checkpoint. Session schemas and settings do not change;
resume uses the saved anchor and complete canonical tree. Cancellation and
failure follow the normal turn-settlement and persistence rollback rules.

## Implementation and verification

| Module | Responsibility |
| --- | --- |
| `src/context/measurement.ts` | Local measurement and exact-prefix usage observations |
| `src/context/lifetime.ts` | Observation lifetime across turns and safe reset boundaries |
| `src/context/diagnostics.ts` | Content-free request and compaction observation channel |
| `src/providers/input-measurement.ts` | Provider wire measurement and opaque reserves |
| `src/context/manager.ts` | Automatic decision, lifecycle, anchor, and diagnostics |
| `src/context/policy.ts` | Model budgets and safe prefix/tail planning |
| `src/context/compactor.ts` | One bounded summary request and acceptance checks |
| `src/context/request.ts` | Final input fit and emergency tool projection |
| `src/context/automatic.ts` | Failure suppression and recovery scope |
| `src/context/projection.ts` | Durable branch-anchor projection |

Focused tests cover full TUI tool sequences, prefix invalidation, raw and opaque
measurement, malformed counts, narrow-window recovery, failed summaries,
deadlines, interruption, and preserved history. `npm run bench:context` also
exercises 12- and 40-read workflows with an inert provider, alongside local
estimation/planning responsiveness. These synthetic checks do not establish
summary quality or live provider latency; use the [validation protocol](../dev/validation/README.md)
for those observations.
