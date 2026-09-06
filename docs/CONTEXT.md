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
Jecode's installed runtime has no tokenizer dependency or extra counting request
on each tool boundary. It includes a checksum-pinned vocabulary as package data;
installation and execution never download it.

`Provider.measureInput` is a local adapter boundary. It measures the same wire
conversion used for generation, counting the outgoing representation once.
Unsent normalized duplicates, foreign raw blocks, and accounting metadata do
not count as prompt text. Providers without this hook use a conservative
normalized-content estimate.

Modern OpenAI API model families with an o200k mapping use an owned byte-pair
counter and OpenAI's `o200k_base` vocabulary. OpenAI Account uses the same
reference encoding, including aliases whose actual encoding is unpublished.
The adapter counts serialized outgoing content with a 10% allowance, plus fixed
protocol/item allowances. This remains an estimate of the request, not an exact
provider count. Unknown or legacy API models, Anthropic, and Ollama retain the
conservative UTF-8 byte, compression, and literal-content estimator.

The tokenizer loads lazily, yields between vocabulary batches and 8,192-code-unit
text chunks, and preserves surrogate pairs at chunk boundaries. Chunk splits
reserve eight additional tokens per boundary. Ordinary user text resembling a
special token stays literal. Completed counts use a 2,048-entry digest-only
cache; repeated chunks use a per-call cache bounded to 131,072 code units.
The implementation and [vocabulary provenance](../dev/context/TOKENIZER.md)
are tested against independent tiktoken fixtures. The vocabulary adds about
1.69 MB of package data and does not introduce an executable dependency.

Both counters support cancellation. Their values are estimates, not billing
counts. Known opaque reasoning fields are not
treated as tokenizable ciphertext when the same assistant response has valid
output usage: the adapter reserves the greater of the visible estimate and
reported output tokens once for that message. Without valid usage, opaque data
retains a conservative byte estimate. Raw protocol data itself is never changed.

Across consecutive turns of an open conversation, the input meter anchors a
successfully sent prefix to the provider's reported input count and adds the
conservative estimate of newly appended content. The observation is valid only while instructions, tools,
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
The tokenizer therefore measures the restored projection afresh, avoiding the
older punctuation-heavy heuristic on the supported routes. No raw provider
payload or calibration metadata is added to the session format.

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

The prompt asks for at most 500 words focused on the active or most recent task:
still-relevant constraints, decisions, exact paths needed to continue, current
changes, final verification results, unresolved work, and next steps. Completed
and unverified work remain distinct. Full code, logs, exhaustive inventories,
and superseded retries are omitted. This is a generation target, not a claim
that the provider enforces a word limit; the bounds below remain authoritative.

The summary request has a 60-second deadline and observes turn cancellation.
The usual local output ceiling is at most 4,096 tokens. Provider transport
limitations still apply: OpenAI Account does not send that ceiling to the
server. Returned summaries must be nonempty and at most 32,768 code units.
Streaming text is also counted and the request is cancelled as soon as it
exceeds that limit, even if the server ignores the output token ceiling.
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
The owned `jecode.context` diagnostics channel emits numeric request measurements,
resolved limits, preparation/provider durations, time to first stream event, and
compaction outcomes. Summary sends also report streamed character count, provider
duration, and time to first nonempty summary text, including unsuccessful sends.
Failed or cancelled provider sends and local preparation
failures are recorded too. Unrelated transport errors never re-enter context
preparation or compaction. No listener is
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
| `src/context/request-observation.ts` | Provider/preparation outcome and timing boundaries |
| `src/context/tokenizer/` | Pinned vocabulary loading, ranked byte merges, bounded text counting |
| `src/providers/input-measurement.ts` | Provider wire measurement and opaque reserves |
| `src/context/manager.ts` | Automatic decision, lifecycle, anchor, and diagnostics |
| `src/context/policy.ts` | Model budgets and safe prefix/tail planning |
| `src/context/compactor.ts` | One bounded summary request and acceptance checks |
| `src/context/request.ts` | Final input fit and emergency tool projection |
| `src/context/automatic.ts` | Failure suppression and recovery scope |
| `src/context/projection.ts` | Durable branch-anchor projection |

Focused tests cover full TUI tool sequences, prefix invalidation, raw and opaque
measurement, malformed counts, narrow-window recovery, failed summaries,
deadlines, interruption, normalized session restart, and preserved history.
Tokenizer fixtures cover ordinary-text counts against tiktoken; a real TUI
fixture verifies restart without premature compact or historical tool execution.
`npm run bench:context` also
exercises 12- and 40-read workflows with an inert provider, alongside local
estimation/planning responsiveness, cold vocabulary loading, and tokenization
of uncached multi-megabyte documents. These synthetic checks do not establish
summary quality or live provider latency; use the [validation protocol](../dev/validation/README.md)
for those observations.
