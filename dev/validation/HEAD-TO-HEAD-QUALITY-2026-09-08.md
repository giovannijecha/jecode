# Output quality and follow-up change — 2026-09-08

## Scope

This record follows the [planning experiment](HEAD-TO-HEAD-PLANNING-2026-09-07.md).
The question is whether Jecode produces correct, maintainable changes and performs
useful verification, rather than merely finishing sooner. The
[quality protocol](../benchmarks/head-to-head/QUALITY.md) was recorded before
detailed artifact review. It separates correctness, scope, maintenance,
verification, execution and the ability to extend existing code.

The six saved cache/planner outputs were copied into randomized task labels,
excluding client names, timings, Git metadata, transcripts and account homes.
Artifact notes were saved before unmasking. This is identity masking, not a fully
blind independent review: the reviewer had seen earlier aggregate results and
the unusual cache-cancellation finding. All original outputs remain unchanged.

Private artifact evidence is under `/var/tmp/jecode-quality-20260908-7e91`.
Live follow-up evidence is under
`/var/tmp/jecode-head-to-head-20260908-quality-47ad`.
Do not publish raw account homes or terminal/session captures.

## Saved-output review

All three cache implementations use explicit generation ownership and isolate
caller cancellation from shared loads. Their service wrapper remains unchanged.
All three planners validate the full graph, use iterative traversal and a
minimum-priority queue, preserve immutable inputs and keep their module boundaries.
No missing required module, runtime dependency or unrelated feature was found.

The supplementary [quality probes](../benchmarks/head-to-head/quality-probes.mjs)
apply the same checks to each saved output. They are post-hoc evidence, separate
from the original 31/57-check acceptance outcomes and original elapsed times.

| Output | Cache schedules | Planner contract probes | CLI hardening probes |
| --- | ---: | ---: | ---: |
| Jecode baseline | 128/128 | 262/262 | 2/2 |
| Jecode grouped | 128/128 | 262/262 | 2/2 |
| Codex | 128/128 | 262/262 | 0/2 |

Each cache seed drives 80 deterministic actions involving callers, cancellation,
clock advancement, invalidation, clear, and old/new load settlement; expectations
come from a separate generation-state oracle. The planner has 256 deterministic
DAG/selection cases checked with a scanning oracle and six invalid ASCII-ID cases.
The broken starting fixtures pass 0/128 and 15/264 respectively; these probes do
not merely accept any implementation. Seeds are descriptive cases, not independent
observations of agent reliability.

### Confirmed differences and limits

- The Codex planner echoes raw terminal control bytes in unknown-flag errors and
  emits an entire 8 KiB argument. Both Jecode outputs avoid those behaviors.
  The 1024-character supplementary cutoff is an explicit review criterion, not
  an exact length prescribed by the original task. Control-byte and long-input
  probes were added after source review and do not change the original score.
- The previously recorded cache probe still distinguishes cancellation inside
  the injected clock callback: both Jecode outputs reject; Codex resolves. This
  remains an unusual, output-derived case, not a universal quality advantage.
- Jecode grouped duplicates adjacency construction between graph validation and
  scheduling; baseline and Codex share a helper. No corresponding functional
  defect was found. Fewer requests did not make every design choice better.
- The native generated tests use deterministic clocks, deferred promises and
  actual CLI processes. Their different totals do not measure equivalent coverage.

### Cross-suite checks

All 18 implementation/test-suite combinations were run in disposable copies.
The reproducible matrix uses the test author's examples/documentation alongside
the other implementation's `src/`. Initial exploratory results with the wrong
example fixture are retained separately and excluded from defect conclusions.

Both Jecode cache implementations pass all three generated suites. Codex fails
the already-known clock-callback case and a test of immediate cancellation after
a cache-hit call returns. The latter requires stronger behavior than the written
contract clearly establishes: a cache hit may have settled before the abort.
Do not count it as an additional proven contract violation.

Planner cross-suite failures fall into two categories: the confirmed Codex CLI
hardening differences, and assertions tied to a particular error message. For
example, one suite expects `unsafe total duration`, another `safe integer`;
both implementations correctly reject overflow. These are not extra functional
defects. Source review and common behavioral checks take precedence over a raw
cross-suite failure count; the original test files were not weakened or rewritten.

## Jecode correction

Reviewing model-facing read feedback found a separate runtime defect:
`read_file` reported `[file is empty]` for an offset beyond EOF, a zero-line
request or a selected blank line in a populated file. The shared empty-string
branch discarded the difference between missing selected text and an empty file.

The corrected reader distinguishes all four outcomes. An EOF notice names the
requested start and observed line count. A zero-line request still validates a
regular file. Normal read content, scan limits, cancellation and stable-path
checks remain in place. Four regressions cover the registered tool path, blank
ranges and zero-range validation. Three false-empty tests failed before the fix
and pass afterward. This removes misleading evidence; it does not quantify how
many model mistakes or seconds the correction will prevent.

## Follow-up setup

Six trials are declared in advance: baseline/Codex/grouped, then
grouped/Codex/baseline. Each gets identical starting files and the same new task
in a fresh session. The common starter was selected by the minimum SHA-256 of
the three complete planner file manifests, before unmasking. It resolves to
the previous grouped output; selection did not use quality or timing.

The new requirement adds completed-task cut points, full validation before
selection, remaining-work durations and repeatable CLI flags. Terminal-safe,
bounded errors are explicitly requested from everyone. The 160-check evaluator
includes the original 57-check suite as a prerequisite plus new examples,
generated graphs, immutable inputs, deep chains and CLI cases. The starter
passes its old contract and 18/160 extension checks; a separate reference
implementation passes 160/160 before live trials. Neither reference nor scorer
is inside a participant workspace.

Environment remains native WSL Linux, Node 22.23.2, pinned Codex CLI 0.153.4,
same account, `gpt-6-astra`, high effort, one controller and serial runs.
Codex Fast mode is off with its default tier; Jecode leaves its optional tier
unset. Actual backend priority is not independently verified. Native prompts,
tools and sandbox boundaries differ as documented in the earlier protocol.

The baseline snapshot contains 485 files, based on `768b686` plus local work;
SHA-256 `59e65822c9577eb7995c69243491b0003632a92b90328739dfea2a0291b87619`.
Both Jecode variants include the read-result correction. Only the experimental
grouping instruction differs between their runtime sources. The production
prompt remains unchanged during measurement. Complete repository checks run
before live timing; only light evidence inspection and documentation work occur
alongside live generation.

The grouped snapshot hash is
`980fcef8df3956d8da3783df403ba0e52a8b9771869f9607803df1a5aeb9c431`.
All nine launches, including preflights, share one WSL boot. Source, harness,
prompt, fixture and evaluator hashes match their frozen manifests. The original
six saved outputs and masked review copies also remain byte-for-byte unchanged.

## Follow-up results

| Configuration | Run | Settlement | Elapsed seconds | Model requests | Reported output tokens | External checks |
| --- | ---: | --- | ---: | ---: | ---: | ---: |
| Jecode baseline | 1 | completed | 420.725 | 27 | 9,374 | 160/160 |
| Codex native | 1 | completed | 352.605 | 11 | 9,939 | 160/160 |
| Jecode grouped | 1 | completed | 331.312 | 10 | 9,169 | 160/160 |
| Jecode grouped | 2 | completed | 333.712 | 11 | 9,378 | 160/160 |
| Codex native | 2 | completed | 394.052 | 10 | 11,588 | 160/160 |
| Jecode baseline | 2 | **failed** | 267.285 until failure | 15, including one failed | 6,320 before failure | 160/160, partial artifact |

Every external check was rerun independently. Ordinary project tests also pass
outside the agent processes: baseline 145/125, grouped 126/124, Codex 125/125,
with zero failures/skips. These totals describe different suites and are not
comparable quality scores. In particular, the failed baseline had not updated
the README or completed final verification: passing its partial code does not
fulfill the whole task.

Source review found bounded changes to selection, argument parsing, help,
tests and documentation. Completed task cut points preserve full-graph
validation and immutable inputs. Existing test changes extend generated cases
and adjust help assertions; no original behavioral requirement was removed.
The five completed outputs have useful checks after their final code changes.
Both Codex summaries accurately disclose their execution limitation below.

The successful baseline fragmented edits, including README updates, across 27
model requests. Grouped runs used 10/11 requests with similar output volume and
finished within 2.4 seconds of one another. This supports the batching mechanism,
but the second baseline failure prevents the planned complete repeated comparison.
Do not average its 267 seconds into successful completion times or discard it
when estimating success. Codex timings additionally include sandbox friction.

### Jecode connection failure

The second baseline failed on request 15 after 14 completed requests. The failed
request lasted 9.138 seconds and reported `transportFailure: closed`, close code
1006, five received messages and 16,697 received characters. No first visible
stream event was recorded. This was not the five-minute progress timeout, a
stream-size rejection or context exhaustion: measured input was about 22,216
tokens against a 258,400-token window. The recording cannot identify the exact
peer/network/client cause of the unclean close.

Across the four Jecode trials, 62 of 63 requests completed, 59 reused a socket,
and there were no HTTP fallbacks, compactions, clipped results or recorder drops.
All issued tool calls have results; all four saved nodes decode with the measured
production codec. Completed runs spent 4.769–10.764 seconds outside the observed
provider-send intervals, including tools, tests and harness overhead. These are
not pure client-overhead or pure inference measurements.

The existing compatibility contract intentionally surfaces ambiguous failures
after generation starts rather than replaying automatically. This investigation
does not remove that protection. Missing visible text is insufficient evidence
that a generation never started: output can already include opaque reasoning.
The runtime now records an allowlisted `responseStage` (`awaiting`, `accepted`,
`output`, `terminal`) for WebSocket sends, without frame content, identifiers or
peer event names. The phase resets for each send and does not affect retries.
These diagnostic changes were made **after** the measured batch and are not
retroactively attributed to it. The original close remains unexplained.

The artifact verifier previously returned success when all file checks passed,
even for this failed turn. It now includes original settlement in its report and
aggregate exit status. Rechecking with the corrected verifier returns 1 because
of the retained baseline failure. The earlier artifact-only verification record
is preserved separately; no original scorer, outcome or elapsed time was changed.

### Execution-environment observation

Both Codex follow-ups reported `spawnSync` returning `EPERM` when CLI tests
captured child output through pipes. The first verified the API, used a temporary
project-local file-capture adapter for CLI tests, removed that adapter, and
disclosed the limitation in its final answer. The native rollout confirms Node
22.23.2, matching the selected executable version. The external evaluator then
passed all 160 checks. The second run used direct/in-process CLI checks and
disclosed that normal full-suite execution remained blocked. Independent normal
project-test reruns outside the sandbox pass in both cases.

Do not silently remove this time, classify it as a generated-code bug, or claim
equivalent process restrictions. Codex retains its native workspace-write sandbox;
Jecode's approved commands are not OS-sandboxed. The precise `EPERM` cause is not
established by the observation alone. No sandbox/configuration change is made
mid-batch. A future environment-control comparison would be a separate experiment.

After the batch, the offline [spawn probe](../benchmarks/head-to-head/spawn-probe.mjs)
reproduced the restriction using the same Node and pinned native Codex binary.
All three stdio modes succeed outside the sandbox. With
`codex sandbox -P :workspace -C WORKSPACE -- NODE PROBE`, `pipe` reports `EPERM`,
while `ignore` and `inherit` succeed. The pipe result even includes status 0;
checking only an exit status would miss the capture error. Native CLI help and
the [configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference)
were consulted to select the diagnostic profile. This identifies a reproducible
execution-boundary difference, not the particular denied syscall or a planner
defect. Resolve or explicitly control it before another timing comparison that
depends on captured subprocess tests.

## Disposition and validation

Keep the grouping instruction experimental and the production prompt unchanged.
The two grouped successes are encouraging, but they do not prove better coding
quality or close the incomplete baseline comparison. Adopt the confirmed
`read_file` correction, improve phase diagnostics, and retain the failed run.

Final checks after runtime corrections:

- Windows Node 24.18: `npm run check` passes; 1,024 tests pass and 11 platform
  cases skip, with no failures. Coverage is 96.21% lines, 88.75% branches and
  94.26% functions. Package/install checks pass with zero runtime dependencies.
- Native WSL Node 22.23.2: typecheck and all 1,035 tests pass, no skips/failures,
  in a separate post-measurement source copy.
- The dev harness passes 20 offline tests. New regressions ensure that a failed
  turn cannot become a successful trial merely because artifact checks pass.
- Socket tests cover closure before acknowledgement, acknowledgement without
  text, opaque output, terminal completion, phase reset on reuse, unknown-event
  privacy, and the existing cancellation/no-replay behavior.
- All 11 temporary account files were removed after trials and checks settled;
  a final scan found none in the laboratory. Production account stores were not
  modified. Local check logs were archived with matching SHA-256 hashes before
  removal from the checkout. Raw evidence remains private.

The next evidence should prioritize unexpected-close recovery and a cleanly
declared repeat after resolving the subprocess-capture restriction. Then test
larger repository changes, long context, interrupted work and independent human
review. The current cache/planner tasks do not establish general superiority,
all-provider reliability, accessibility or release-candidate soak acceptance.
