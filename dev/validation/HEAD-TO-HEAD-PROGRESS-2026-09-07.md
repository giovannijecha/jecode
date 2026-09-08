# Responses progress deadline follow-up, 2026-09-07

This follows the [six-run baseline](HEAD-TO-HEAD-2026-09-07.md), whose failed
Jecode attempt remains part of the evidence. It tests a specific local timeout
fix and repeats the same task; it is not a general product ranking or release gate.

## Confirmed cause and change

The baseline's third Jecode run stopped after 120 seconds without an application
event. This was a local transport deadline, not evidence of a broken connection.
The completed implementation passed the external functional checks, but the
agent never finished the requested tests, documentation and final review.

Both OpenAI routes now use the existing five-minute substantive-progress budget
on HTTP and WebSocket without a separate two-minute event cutoff. State-only
keepalives do not renew that budget. Other providers retain their existing
two-minute event bounds. A local stream timeout has its own normalized failure
kind and user-facing explanation. Handshake limits, cancellation, complete
history and the prohibition on replay after partial generation remain intact.

Eight regression cases exercise real loopback transports with a controlled
monotonic clock and timers. They reproduce a 150-second silence between reasoning
events for API and Account, reach the five-minute deadline despite keepalives,
and cancel during a long silence without another generation or HTTP replay.
The sparse-event case failed against the previous implementation and passes
after the fix. This does not establish that the old upstream request would
eventually have completed.

## Frozen candidate and protocol

- Base commit: `768b686934184627cac3d7e30773f124405ae56f` plus local changes.
- Candidate SHA-256: `eaa68d40961f9cbc80a676aa155402cd447d4d6a9ea3e76940550ff49a629f96` (444 files).
- Native Codex 0.153.4, Node 22.23.2, Ubuntu in WSL2, same account/model/high effort
  and Linux storage as the baseline. Codex's existing pinned installation was
  copied into a new private laboratory; neither source snapshot was overwritten.
- Runtime changes since the baseline are limited to seven files owning progress
  deadlines and failure normalization/presentation. No model prompt, effort,
  output budget, tool schema, context policy or fixture changed.
- Four new fresh runs were declared in advance: Jecode 1, Codex 1, Codex 2,
  Jecode 2. One batch parent keeps the guest alive. Native TUI preflights passed
  before any task was submitted.
- The same prompt, original fixture and corrected 56-check evaluator are used.
  Timing, security, telemetry and backend scheduling limitations remain those
  of the baseline protocol. Full repository gates ran before live task timing.

## Supplementary correctness probe

`explore-ledger.mjs` adds 422 deterministic checks independently of the original
scorer: 400 generated ledgers plus 22 literal boundary cases. These are explicitly
post-hoc checks; they never alter the recorded 56-check outcomes or task times.
The initial fixture passes only 1/422. Seed: `1279607879`; probe SHA-256:
`dc95c33f58180dbc978f187940da961590858190ae586c82d5eed2e04e459a74`.
All six saved baseline implementations pass 422/422, including the unfinished
Jecode run. That run remains failed: functional behavior alone does not fulfill
the requested tests, documentation and review.

## Automated verification

- Frozen Linux candidate: typecheck passed; 1,029 tests passed, zero skips/failures.
- Windows checkout: typecheck, test and complete check gates passed. 1,029 tests,
  1,018 passed, 11 platform skips, zero failures. Coverage: 96.20% lines,
  88.64% branches, 94.26% functions.
- Package verification: 192 files, 1,928,000 bytes, zero runtime dependencies;
  installed CLI smoke checks passed. These are local checks, not a new CI matrix.

## Live results

All four declared attempts completed. No attempts were retried, replaced or
discarded. The clock and independent evaluation boundaries match the baseline.

| Run, in execution order | Task seconds | Requests | Reported output tokens | External checks |
| --- | ---: | ---: | ---: | ---: |
| Jecode 1 | 442.988 | 15 | 12,139 | 56/56 |
| Codex 1 | 357.170 | 12 | 9,963 | 56/56 |
| Codex 2 | 538.563 | 16 | 15,403 | 56/56 |
| Jecode 2 | 439.274 | 18 | 11,569 | 56/56 |

Jecode's arithmetic mean is 441.131 seconds (7:21); Codex's is 447.867 seconds
(7:28). The difference is 1.50% in Jecode's favor in this sample, with only two
observations per client and substantial Codex variation. This is not evidence
of a general speed advantage, a stable variance difference or statistical
equivalence. The old failed Jecode attempt remains in the separate baseline;
pooling different versions or dropping that failure would misrepresent the test.

| Run | Request-boundary seconds | Seconds outside boundary | Initial preparation, seconds |
| --- | ---: | ---: | ---: |
| Jecode 1 | 438.995 | 3.993 | 1.173 |
| Codex 1 | 353.341 | 3.829 | Not comparable |
| Codex 2 | 532.364 | 6.199 | Not comparable |
| Jecode 2 | 435.244 | 4.030 | 0.738 |

Jecode spent about 99.1% of each task inside provider send/stream callbacks.
This boundary includes local callbacks as well as network and provider work;
it is not pure inference time. Preparation medians were 3 and 1.5 ms. There is
no large unaccounted pause outside requests in these two runs. Changing model
work volume or request sequencing has more potential here than shaving a few
milliseconds off local tool dispatch.

Both Jecode runs used WebSockets throughout: 14/15 and 17/18 requests reused the
connection and sent verified incremental input. All 31 and 34 tool calls had
matching results, with no tool errors. There were no HTTP fallbacks, compactions,
clipped results, transport failures or dropped diagnostic records. Reported
input/cache totals were 279,669/241,280 and 323,479/288,640 tokens; these include
repeated context and do not represent unique context size or a billing estimate.

Codex completed 11 and 15 model-facing tool calls with matching results. All
12 and 16 response windows paired unambiguously; usage sums matched the native
cumulative session records. Telemetry reported zero errors. Three and four
API events without timestamps were excluded from timing, as in the baseline.
Input/cache totals were 273,956/246,144 and 462,030/368,896 tokens. Nested tool
durations were not added to their enclosing model-facing `exec` calls.

In Jecode 1, request 6 completed in 186.272 seconds with its first semantic tool
announcement at 186.229 seconds; it produced 6,014 output tokens for tests and
README edits. In Jecode 2, request 6 completed in 175.501 seconds with its first
semantic event at 175.423 seconds. The recorder does not time every wire event:
these are long valid requests, not proof that either contained a continuous
120-second protocol silence. The deterministic transport regressions establish
the old cutoff defect; the live runs establish completion on the new candidate.

## Remaining measured opportunities

- **Edit planning and request granularity.** Jecode 1 used requests 10-13 for
  four small independent changes in the same test file: dead test-branch removal,
  an invalid decimal case, exact running-sum boundaries and separate excluded-row
  filter checks. These requests consumed 36.218 seconds and 571 output tokens,
  with no intervening read or test execution. The controller already accepts
  multiple calls and orders writes. This is an agent planning opportunity, not
  evidence that the controller serializes independent reads incorrectly. A future
  prompt/tool-ergonomics experiment should group related corrections while retaining
  validation; it must measure the resulting time rather than claim all 36 seconds
  as recoverable. These changes include useful checks and should not simply be removed.
- **Initial preparation.** The first 0.738-1.173 seconds include context metadata
  and input measurement. A separate post-run context benchmark measured cold
  tokenizer setup at 158.527 ms, so the whole initial delay cannot be attributed
  to tokenization from these observations. Profile metadata and measurement
  separately before introducing background initialization or new machinery.
- **Task diversity and recovery.** Both clients pass the available functional
  checks. This task cannot establish a quality advantage. Add held-out debugging,
  a larger repository and multi-turn interruption/resume/context tasks before
  tuning planning or claiming better daily-use behavior. Account results are not
  live validation of OpenAI API, Anthropic API or Ollama API.

## Final independent checks

- Source, prompt, initial fixture and evaluator hashes match all four manifests.
  Every workspace retains its single synthetic Git baseline and zero dependencies;
  no links were found. Changes are confined to source, README and added tests.
- The 56-check scorer passed again on every saved project. The separate 422-check
  probe also passed on all four; it remains a post-hoc supplement, not a changed
  benchmark score. Focused source review found no additional actionable functional
  defect in these implementations.
- Each project's own tests passed independently: Jecode 175 and 126, Codex 139
  and 134, with zero failures/skips. Test counts are not comparable quality scores.
  All final replies, test files and READMEs are present. Both Jecode checkpoints
  decode through the measured source's production codec.
- The Linux harness's ten checks passed. A post-run context probe passed its
  existing time/stall bounds; the 12-read scenario had no compaction and the
  40-read scenario had one. These are synthetic checks, not a long live session.
- Eight temporary account files were removed after the batch. Production account
  stores were untouched. Private raw measurements remain in the active Linux
  laboratory. No credentials, transcripts or generated projects ship in the repo.

The live source stayed frozen throughout. The supplementary probe and this report
were added afterward to the working checkout and are not part of the measured
444-file snapshot. Light inspections occurred during the serial runs; expensive
repository gates preceded timing, and project test reruns and the context benchmark
followed the completed batch. No new CI matrix, PR, release or registry publication
was performed for this follow-up.
