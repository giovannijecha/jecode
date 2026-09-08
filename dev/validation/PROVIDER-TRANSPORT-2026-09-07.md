# Provider transport development validation — 2026-09-07

Local development evidence for `perf/provider-roundtrips`, based on
`768b686934184627cac3d7e30773f124405ae56f`. The package version remains 0.8.7;
these working-tree changes are not a published release or an accepted candidate.

## Automated evidence

Environment: Windows, Node 24.18.0. `npm test` passed; the final `npm run check`
passed with 1,010 passing tests, 11 platform skips, no failures or cancellations.
Coverage: 96.20% lines, 88.62% branches, 94.31% functions. The source, type, package,
zero-runtime-dependency, and isolated CLI installation gates passed.

The transport, context measurement, metadata, diagnostics, and controller
scheduling/recovery subset also passed on the minimum Node 22.18.0: 60 tests,
no failures, cancellations, or skips. The remote OS/Node CI matrix was not run
as part of this local record.

| Boundary | Observed behavior | Evidence |
| --- | --- | --- |
| Responses continuation | Authenticated socket reuse; only new input sent after an exact matching prefix; complete caller input retained. | [Session tests](../../test/responses-session.test.ts) |
| Invalidation and recovery | Full input after changed settings, context, authorization, or reconnect; one full retry for an explicitly missing predecessor. | [Session tests](../../test/responses-session.test.ts) |
| Ambiguous failures | No HTTP replay after streamed progress or cancellation; rejected upgrades use HTTP without sending a socket generation. | [Session tests](../../test/responses-session.test.ts) |
| Socket ownership | Bounded decoded events and queues; idle/cancelled reads stop; redirects do not forward authentication; closing ends TCP even when the peer ignores close frames. | [Socket tests](../../test/websocket.test.ts) |
| HTTP completion | A terminal response settles before EOF; bounded tails drain without blocking the caller; sequential generations can reuse connections. | [Completion tests](../../test/stream-completion.test.ts) |
| Shared reads | The fifth read starts before a slow first call finishes; four-call concurrency cap, ordered results, and exclusive barriers remain intact. | [Scheduling tests](../../test/controller-scheduling.test.ts) |
| Ollama metadata | Stable capacity reused across tool round trips; missing data retried after short expiry; cancellation and key changes remain effective. | [Cloud tests](../../test/ollama-cloud.test.ts) |
| Diagnostics | Whitelisted timings, transport, sizes, usage, and native network codes survive; private content and malformed fields do not. | [Observation tests](../../test/context-request-observation.test.ts) |

The minimum-runtime check exposed synchronous re-entry during a rejected native
WebSocket close on Node 22. Closing is now idempotent. Review also identified
that the native public close method only initiates a closing handshake. A small
standard-library upgrade dispatcher retains socket ownership for deterministic
termination while leaving WebSocket framing to Node.

Initial WebSocket error events retain validated status and error codes for the
existing account-refresh and context-overflow recovery paths. Once an event has
been forwarded, a later error cannot authorize those automatic retries.

The HTTP pool may allocate a second connection while a previous response tail
drains. Tests require actual reuse and nonblocking completion, not an exact
pool size or a timing percentage under concurrent test-runner load.

## Live OpenAI Account evidence

The user's source-checkout recording from 12:14–13:02 UTC contains 57 requests:
56 completed and one failed, all using WebSocket, with zero dropped records.
There were two connection establishments and 55 requests reused the connection
with incremental input. Total outgoing observed JSON was 1,379,056 bytes. Local
preparation had a 6 ms median and 12 ms p95; reused-channel send boundaries had a
1 ms median and 2 ms p95. Those send timings do not measure network round-trip
latency or model throughput and are not a comparison with another client.

No compaction or result clipping occurred. The failed request's calibrated input
was 165,418 tokens against a 258,400-token effective window. It failed 69,762 ms
after sending, after its first thinking event at 14,142 ms. The latest saved
turn passes the session codec: 42 calls, 42 results (one tool error), no unmatched
calls. The transport had already reconnected earlier in the recording; this was
not a single connection running for nearly an hour.

The earlier recording, initially reported as empty by directory-listing metadata
before its contents were read, also confirms source WebSocket use. Its failed
request lasted 123,030 ms without
visible output. The prior attribution to the separately observed global 0.8.7
process was incorrect. Both recordings contain only a generic failure outcome;
neither proves the specific socket/timeout/protocol cause retrospectively.

A loopback reproduction exposed that malformed or oversized WebSocket events
were incorrectly classified as network failures because their messages contained
"socket". Structured transport errors now preserve that distinction, close codes,
and received-event sizes through provider normalization. Tests cover failure
after thinking starts, private close reasons, and absence of automatic replay.
This corrects diagnostics and UI classification; it does not establish that the
live interruptions are fixed. A recurrence captured with the new diagnostics is
still needed to identify their cause. No limits or retry policy were relaxed based
on this incomplete trace.

### Completed follow-up turn

The interactive-demo task ran from 13:54:34.910 to 14:13:26.815 UTC
(15:54–16:13 Europe/Rome), using OpenAI Account, `gpt-6-astra`, high effort.
All 28 requests completed over WebSocket. The saved node passes the production
session codec and has a completed settlement: 28 assistant messages, 58 tool
calls, 58 matching results, no duplicate call IDs or unmatched calls, and no
failure notices. No compaction or result clipping occurred.

| Measurement | Observed value |
| --- | --- |
| Turn elapsed time | 18 min 51.905 s |
| Provider request time, sum | 17 min 6.163 s (90.66% of elapsed time) |
| Four command executions, sum | 1 min 44.124 s (9.20%) |
| Local request preparation | 226 ms total; 7.5 ms median; 11 ms p95 |
| First normalized output event | 10.067 s median; 16.685 s p95; 26.382 s maximum |
| Longest request | 128.343 s; first event at 14.773 s; 4,027 reported output tokens |
| Connection reuse | One full send, then 27 incremental sends on the same connection |
| Outgoing request JSON | 910,476 bytes total; 647,345 initial, 263,131 incremental |
| Reported input cache usage | 5,183,360 / 5,243,305 tokens (98.86%, weighted) |
| Final reported input | 210,316 / 258,400 effective context tokens (81.39%) |
| Final calibrated projection | 210,858 tokens; 21,446 below the 232,304-token trigger |

Provider time includes network, provider waiting/thinking/output, and client
stream processing; it does not isolate server inference. The remaining 1.618 s
after provider and command totals is an accounting remainder, not a direct CPU
overhead measurement. Shared-read durations overlap and include ordered delivery,
so summing them would not measure elapsed I/O time. Reused-channel send boundaries
were 2 ms, which does not establish a 2 ms network round trip. First output means
the first normalized text, thinking, or tool event, not hidden reasoning onset.

The conservative standalone estimate for the last request was 265,348 tokens.
The verified provider-prefix calibration stayed close to actual input instead:
210,858 projected versus 210,316 reported. This avoided an unnecessary early
compaction without raising capacity or reducing the requested reasoning effort.
The largest decoded event was 103,297 characters; no stream-bound failure was
recorded. These observations do not provide a controlled comparison with Codex.

One command failed during development: a focused browser run passed 14 tests and
failed the same unavailable-storage scenario on desktop and mobile. The test used
`check()` while expecting the application to roll the checkbox back to unchecked;
the subsequent patch used `click()` while retaining the rollback assertion.
Two later full checks passed 18 Node tests and 46 browser tests each. Review fixes
and regression cases between those checks addressed a pending Deny choice on
resume and cleanup of an old demo instance. These are recorded command results,
not tests rerun during this analysis. Safari, Firefox, and manual screen-reader
behavior remain unverified.

Reproduction scope: request sequences 9–36 in the local recording
`context-1788787238608-4949e6f1-7406-448d-a0b8-8a931fd317bb.jsonl`, selected by
the saved node's creation/update timestamps, match its 28 assistant messages.
The earlier turn and the idle gap between turns are excluded. At inspection the
file contained 36 valid consecutive records (25,097 bytes), despite directory
metadata reporting zero bytes. No recorder end marker was present, so a final
dropped-record count is unavailable. Median uses the middle-pair mean; p95 uses
nearest rank; cache usage is the ratio of sums, not a mean of percentages.
SHA-256 of the 36-line recording snapshot:
`1904390b2f35962168008f41e6051d2f75b0500559682509ffc6c258eea23ed8`.
SHA-256 of the analyzed `000006.json` node snapshot:
`e5c10f0b29dfeba1f01c5c1e8f8cdabf860c1a32a47629186e385012950b6024`.
Raw recordings and session content remain local and untracked. This successful
turn adds live evidence; it does not establish the cause or resolution of the
earlier interruptions.

## Live verification still required

API-route generations, controlled real-account performance comparisons, terminal
latency measurements, and a completed soak period remain unverified. Loopback
fixtures do not establish provider latency or an improvement percentage.

Use the [context recorder](../context/README.md) from a disposable workspace:

1. Run a multi-tool turn with OpenAI API and OpenAI Account separately. Check
   `transport`, `reused`, `incremental`, `requestBytes`, and visible-event timings.
2. Interrupt and continue, resume the conversation, and compact context. Confirm
   complete context is resent when needed and historical tools never execute.
3. Verify HTTP fallback in an environment that rejects WebSocket upgrades.
4. Exercise Anthropic and Ollama streaming, checking reported cache usage and
   metadata behavior without changing model, effort, or service tier.

Account access remains experimental. Fast service tiers, longer paid cache TTLs,
and HTTP request compression are not enabled by this change. Preserve existing
quality and pricing settings when comparing traces.

Wire references: [OpenAI Responses WebSocket mode](https://developers.openai.com/api/docs/guides/websocket-mode),
[Codex transport](https://github.com/openai/codex/blob/main/codex-rs/core/src/client.rs),
and the [Undici dispatcher contract](https://github.com/nodejs/undici/blob/main/docs/docs/api/Dispatcher.md).
