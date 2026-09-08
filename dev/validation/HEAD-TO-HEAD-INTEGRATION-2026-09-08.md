# Integration and controlled execution comparison — 2026-09-08

## Scope and controls

This follows the [quality investigation](HEAD-TO-HEAD-QUALITY-2026-09-08.md)
under the [integration protocol](../benchmarks/head-to-head/INTEGRATION.md),
recorded before the six timed trials. Evidence stays private under
`/var/tmp/jecode-head-to-head-20260908-integrated-bc71`.

Both clients use native WSL, Node 22.23.2, the same OpenAI Account,
`gpt-6-astra`, high effort and a single controller. Codex CLI remains pinned to
0.153.4 with Fast mode off and the default service tier; Jecode requests no paid
priority. Actual backend priority is not independently verified.

The previous pipe-capture error reproduces under Codex's offline workspace
profile on Node 22.23.2 and 24.20.0. Enabling arbitrary Unix sockets alone does
not remove it; enabling command networking does. The new isolated profile keeps
workspace filesystem permissions and enables networking, which Jecode's approved
commands already permit. The filesystem boundaries still differ. No personal
configuration is changed. The precise denied syscall is not established.

The named profile is selected with `default_permissions`, without legacy
`sandbox_mode`, following the
[configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference).
An initial preflight used the sandbox subcommand's flag on the interactive CLI
and was rejected before any task was submitted; it remains a recorded setup
failure. The corrected strict-config preflight passes. All three stdio probes
and the unchanged starter's 116 own tests pass inside the selected native
sandbox before timing.

The planner-progress starter, prompt and 160-check evaluator are unchanged from
the prior follow-up. Its selected starter manifest hash remains
`11661ff6ba4a0141f887f79bbd830d391f45512bfa5fb65f34695158064ba3a2`.
Every trial starts fresh. The declared order is baseline/Codex/grouped, then
grouped/Codex/baseline. Completion timing starts at the ready composer and excludes
setup and independent external verification. Failures remain failures even if
their partial artifacts pass tests.

Baseline contains current runtime corrections with the original planning prompt:
490 source files, SHA-256
`c2be2754730c22421ff2b3ba927750cbc8c1fa7a86a6da4c3714cf3f8f878c7c`.
Grouped changes only the four-line planning instruction; SHA-256
`42bdc756befb69ac702ce6edb72236854ecd50fb616f4f409138b57a5291328a`.
Both snapshots are based on `768b686` plus local work. The corrected harness
configuration is frozen separately in each trial's Python file hashes; runtime
snapshots retain the earlier setup helper as source evidence.

## Transport correction and recovery evidence

A native WebSocket error after a successful open was classified as an initial
connection failure. It now reports a stream disconnection, preserves the bounded
native cause when available, and tells the user to send a message to continue.
Initial connection failures retain their existing classification and fallback.
No generation replay is added.

The new integration regression drives the production TUI, controller, wire
assembler and session store against a loopback WebSocket. One tool effect completes;
a later response sends a complete tool item and partial text, then disconnects
without a terminal response. That second call never executes. Exit/resume makes
no provider request; explicit continuation opens a new socket with full saved
context, excludes uncommitted output and preserves the old failed node. The
original effect is recorded exactly once.

A separate transport test reuses one socket for 32 fragmented UTF-8 messages,
with ping frames interleaved and TCP chunks splitting headers/code points.
It passes on both supported Node lines. These tests validate the exercised
protocol/recovery paths, not the unknown cause of the previous live close 1006.

## Results

All six declared trials completed and passed 160/160 frozen external checks.
Independent reruns of their own tests also pass, without skips. These are
separate checks after timing, not the agents' self-reported test results.

| Trial | Elapsed seconds | Model requests | Output tokens | Independent own tests |
| --- | ---: | ---: | ---: | ---: |
| Jecode baseline 1 | 439.913 | 29 | 9,733 | 126 |
| Codex 1 | 315.318 | 9 | 8,782 | 126 |
| Jecode grouped 1 | 330.053 | 11 | 9,088 | 125 |
| Jecode grouped 2 | 344.440 | 13 | 9,147 | 146 |
| Codex 2 | 341.012 | 10 | 9,466 | 127 |
| Jecode baseline 2 | 356.517 | 20 | 8,733 | 125 |

| Client/variant | Mean seconds | Mean requests | Mean output tokens |
| --- | ---: | ---: | ---: |
| Jecode baseline | 398.215 | 24.5 | 9,233 |
| Jecode grouped | 337.246 | 12 | 9,117.5 |
| Codex | 328.165 | 9.5 | 9,124 |

Grouped uses 15.3% less elapsed time and 51.0% fewer model requests than baseline
in this batch. Both grouped runs finish faster than both baseline runs. Codex
remains 9.081 seconds ahead on average: grouped Jecode takes 2.8% longer. This is
two observations per variant on one repeated task, not a statistically established
ranking or a measurement of independent decoder speed. Earlier failure-containing
batches remain separate; their latencies are not pooled into these means.

Jecode completes 73/73 requests, with 69 socket reuses, no transport failures,
fallbacks, compactions, clipped results, unmatched tool calls or recorder drops.
Grouped's mean time outside the measured provider adapter is 5.237 seconds
(1.6% of its elapsed time), including tools and local coordination. The adapter
interval includes network, generation and event handling; it cannot separate
server inference from local parsing. These data point to avoiding unnecessary
model round trips, rather than a large unaccounted local delay.

Codex has nine and ten unambiguously paired task response windows. Task input,
cache, output and reasoning usage reconcile with both saved rollouts. During
analysis, a prewarm request in Codex 2 was found to start before submission and
complete after it. Timestamp filtering alone counted its completion as an
eleventh task response. The corrected analyzer pairs the full sequence before
filtering, excludes this one pre-submission completion from task usage, and keeps
the original 341.012-second elapsed time, including overlapping warmup wait.
Original telemetry and the earlier derived reports remain archived; the measured
harness and trial outcomes were not edited. Unknown boundaries are not excluded.

The first baseline used 29 model requests, including seven consecutive README
edit responses totaling 83.999 seconds of observed provider time. The first
grouped trial used 11 requests and applied 28 edits in three edit-bearing
responses. Its initial test failure was an outdated help-text assertion after
adding the new flag; it corrected the assertion and reran verification. This
intermediate red check is not a final failed task or a transport failure.

## Output and process review

The independent shared planner probes pass 264/264 for every output, covering
seeded graph cases, identifier handling and CLI safety. Review of final diffs,
tests and verification order finds the expected API/CLI completion feature,
full-graph validation before selecting remaining tasks, immutable input handling,
safe bounded error rendering, and relevant README/help updates. All outputs
retain zero dependencies and the single starter commit, with no workspace links.
Own test counts vary because the generated suites differ; they are not quality
scores. All six agents perform verification and final inspection.

Review of the first baseline also motivates a separate, supplementary
[representation probe](../benchmarks/head-to-head/representation-probe.mjs):
frozen tasks with non-enumerable fields, already accepted without completion.
The baseline deliberately preserves those fields when creating remaining tasks;
object spread in other outputs drops `id` and `duration`. The probe was added
after observing that baseline and is not part of the frozen 160-check score.

| Output | Additional representation cases |
| --- | ---: |
| Jecode baseline 1 | 3/3 |
| Jecode baseline 2 | 2/3 |
| Jecode grouped 1 | 2/3 |
| Jecode grouped 2 | 3/3 |
| Codex 1 | 2/3 |
| Codex 2 | 2/3 |

The four failures affect completion with non-enumerable fields; the same object
works when completion is omitted or empty. Baseline 1 and grouped 2 independently
identify and cover the issue during their own review. This is a concrete remaining
quality weakness in some generated outputs, not evidence of systematic superiority
for either client or proof that grouping causes the weakness. No saved output is
patched to improve its score. The review is source-informed, not blind, and these
cases were not specified before the batch.

## Integration decision and provenance

Adopt the exact four-line grouping instruction exercised here. Both candidate
turns complete, preserve the frozen contract and meaningful verification, and
show no material regression within that contract. The additional representation
gap stays visible as a limitation shared by some baseline and grouped outputs.
No task-specific hints, tool-count targets, effort reduction or extra model worker
are added. Reads still establish changes, dependent edits wait for results, and
the controller continues to serialize writes and commands.

After promotion, all 184 runtime TypeScript files in the checkout are byte-for-byte
identical to the measured grouped snapshot, with no extra runtime files. The
verifier confirms unchanged source, prompt, fixture and evaluator in every trial,
and no missing declared run. All six manifests record the same harness hashes,
environment and WSL boot ID `b9d8845d-cc5f-4976-ae58-8618b95454ce`. Both Codex
rollouts confirm Astra/high, workspace writes and enabled command networking.
All 18 frozen harness files still match their recorded hashes. All four saved
Jecode nodes decode with the measured production codec.

Private evidence includes `comparison.json`, `planning-report.json`,
`verification.json`, `supplementary.json`, raw trials and the analysis audit.
Corrections to post-run analysis are separately hashed under `analysis/`, outside
the frozen execution harness. Eleven temporary account files were removed after
all runs settled; the final recursive audit finds zero account copies in the lab.
Original account stores are unchanged.

The next quality comparison should use held-out bug fixes and changes that stress
existing API contracts, with evaluators frozen before either client runs. This
batch supports the grouping improvement, not a general claim of better output
than Codex, validation of API routes, or elimination of intermittent close 1006.

## Validation

- Before timing, Windows Node 24.18: complete `npm run check` passes; 1,026 tests pass and 11
  platform cases skip, zero failures. Package/install checks pass.
- Before timing, native WSL Node 22.23.2: grouped candidate typecheck and all 1,037 tests pass,
  no skips or failures.
- The finalized harness passes 22 offline tests, including opt-in permissions metadata,
  protection against inserting the grouping instruction twice and failed-turn
  settlement, missing declared trials and prewarm attribution in analysis.
- After integration, the Windows source and type gates pass. An initial complete
  check reports `websocket.test.ts` as a failed test process after 69 ms, without
  individual test results or an error cause. Its six cases pass in isolation;
  the complete coverage rerun with TAP diagnostics passes 1,026 tests with the
  same 11 platform skips, without changing runtime or tests. The process failure
  is not reproduced or explained; retain it as unresolved test-run instability,
  separate from the live transport observations.
- Final Windows coverage is 96.22% lines, 88.76% branches and 94.26% functions.
  Source-tree, package and isolated CLI installation gates pass after that rerun:
  192 package files, 1,928,894 unpacked bytes, zero runtime dependencies. Original
  failed and passing logs remain private alongside the pre-measurement results.

This record is development evidence, not release-candidate soak acceptance.
