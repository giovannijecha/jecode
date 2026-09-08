# Codex and Jecode live comparison, 2026-09-07

This is development evidence for a frozen Jecode source snapshot, not a release
acceptance record or a general client ranking. The reproducible
[laboratory and task](../benchmarks/head-to-head/README.md) live outside the
installed runtime. The investigation remains active for follow-up comparisons.

## Environment and controls

- Ubuntu 26.04.1 LTS in WSL2, kernel
  `6.18.33.2-microsoft-standard-WSL2`; 24 visible CPUs and 16,080,576 KiB RAM.
- Native Linux storage under a private `/var/tmp` laboratory. Executables,
  projects and data homes run inside Linux; task I/O does not cross `/mnt/c`.
- Node.js 22.23.2 and Python 3.14.4; native Codex CLI 0.153.4 installed from the pinned npm package.
  The initially discovered Windows command shim was excluded.
- Jecode package version 0.8.7 plus the local transport changes based on commit
  `768b686934184627cac3d7e30773f124405ae56f`. This is not registry 0.8.7.
  The immutable 421-file snapshot passed typecheck and all 1,021 Linux tests.
- Same authorized OpenAI Account identity, requested `gpt-6-astra`, high effort,
  new sessions, identical prompt and independent fixture copies. Account equality
  was checked in memory; no account identifier or credential belongs in this report.
- One model-facing controller per client. Codex delegated agents, memories,
  plugins, apps and web access were disabled. Each native prompt/tool contract
  remained intact, including Codex's host-side `exec` wrapper.
- Codex Fast mode was disabled and its tier set to `default`; Jecode left the
  optional tier unset. Actual backend routing/priority was not independently
  verified. Equal account/model/effort is not proof of identical server scheduling.
- Both real TUIs ran in a 140-by-40 PTY with `NO_COLOR`. Codex used workspace-write
  sandboxing and no approval prompts. Jecode used its real session permission
  menu to allow edit/write/command access. Its approved shell is not OS-sandboxed.

Snapshot SHA-256:
`f9247e63a4ee0eeccdfaf264bc3a15a4217e264e6d1a8c46a952bc6940951cc4`.
Prompt SHA-256:
`e7c5256707a052a314082982b6c3a683aa38b7ad120840c385f9c5b10e78776f`.
Codex installation lock SHA-256:
`16018c3a5d80e18ea7157a785a63bc7d3f119358e36d4dd28fe0476e78321673`.

## Task and measurement

The task hardens a small CSV ledger CLI: quoting/newlines, real calendar dates,
exact integer cents and overflow, account/date filtering, CLI errors, regression
tests and documentation. The unchanged initial project passed 9 of 56 independent
acceptance checks. The evaluator stayed outside each model's workspace.

Runs were serial in the order Jecode 1, Codex 1, Codex 2, Jecode 2, Jecode 3,
Codex 3. The clock starts at prompt submission after startup and permissions;
it ends when the observer detects a settled checkpoint or native completion
notification. Failures retain their elapsed time and state. Acceptance runs
afterward and is not included in task time. A failed run is not a fast success.

WSL's monotonic/realtime clock offset changed between the first pair and the
repetitions, indicating a guest restart. Versions and frozen inputs stayed fixed;
OS cache warmth did not. The last two runs use one batch parent that keeps WSL
alive. Future comparisons should run the entire declared sequence this way.
No agent runs or full repository gates overlapped. Light local inspections and
collector checks occurred during the experiment; the host was not a dedicated
performance runner, and upstream load/cache behavior was not controlled.

During evaluator review, two CLI assertions were relaxed from byte-identical JSON
to semantic JSON equality plus a final newline. Property ordering was not in the
task contract. Both first solutions had already passed the stricter version and
passed the corrected evaluator again. Original outcomes were preserved.

- Original evaluator: `89fc2090376fb4afd662bb3d1a2b2024dc4a4ea7800e15fbece98be3a3ae802a`.
- Corrected evaluator: `88814d2edc40f5d65fb123e1162278bb5f79d53ca30f9955c73c256b0f6b849b`.

## Results

| Run | Outcome | Task seconds | Provider requests | Reported output tokens | External checks |
| --- | --- | ---: | ---: | ---: | ---: |
| Jecode 1 | Completed | 381.900 | 15 | 10,572 | 56/56 |
| Codex 1 | Completed | 566.598 | 10 | 16,879 | 56/56 |
| Codex 2 | Completed | 449.162 | 12 | 12,653 | 56/56 |
| Jecode 2 | Completed | 437.311 | 17 | 11,583 | 56/56 |
| Jecode 3 | Failed: idle timeout | 274.156 | 7, including 1 failed | 3,580; failed response unreported | 56/56 |
| Codex 3 | Completed | 577.083 | 15 | 16,687 | 56/56 |

Jecode completed 2 of 3 attempts; Codex completed 3 of 3. Successful Jecode runs
took 6:22 and 7:17, with a conditional median of 6:50. Codex took 9:27, 7:29 and
9:37, with a median of 9:27. These medians condition on unequal successful samples;
they are not evidence of an overall Jecode speedup. The failed Jecode run is
neither omitted nor included as a successful low-latency result. No live task was
retried to replace it. Three observations also do not estimate a stable failure rate.

| Completed run | Time inside request boundary, seconds | Time outside that boundary, seconds | Jecode preparation, seconds |
| --- | ---: | ---: | ---: |
| Jecode 1 | 378.267 | 3.633 | 1.232 |
| Jecode 2 | 433.700 | 3.611 | 0.850 |
| Codex 1 | 563.370 | 3.228 | Not comparable |
| Codex 2 | 444.379 | 4.783 | Not comparable |
| Codex 3 | 574.273 | 2.810 | Not comparable |

The request intervals dominate completed task time. No large additional gap
between Jecode requests is evident here. Its preparation median was 3 ms in run 1
and 2 ms in run 2; the first request included 1,195 and 814 ms respectively.
Callbacks inside the provider boundary are not separately profiled, so this does
not rule out every possible local bottleneck. Provider response time also varies
with the quantity and sequence of generated work: completed Jecode runs reported
less output than each Codex run, even though they used more request rounds than
the first two Codex runs.

All Jecode requests used WebSockets, with 14/15, 16/17 and 6/7 connections reused
and incremental continuations. No HTTP fallback, compaction or tool-result clipping
occurred. All 26, 27 and 13 issued tools respectively had matching results, with
no tool errors. Diagnostic recorders dropped zero records. Codex canonical tool
calls/results matched at 9, 11 and 14 model-facing `exec` calls; nested operations
were counted separately. Its telemetry collectors reported zero errors, and
response-usage sums matched native cumulative session usage in all three runs.
API-request telemetry without event timestamps was excluded from task timing;
the analyzer retains those counts rather than assigning batch arrival times.

## Independent verification

- All six saved projects passed the corrected 56-check evaluator. Each project's
  own test runner was independently rerun and passed. Jecode 3 had only the
  original smoke test, confirming that its test/documentation work was unfinished.
- Source snapshot contents, initial fixture hashes and prompt hashes matched for
  every run. Git history stayed at the one synthetic baseline commit, dependencies
  remained empty, and no workspace symlinks were found.
- All three Jecode checkpoints, including the failed turn, decoded through the
  measured runtime's production session codec. Issued tool/result identities match.
- Source review of the completed implementations found separate CSV parsing,
  field/filter validation and CLI handling, exact cents conversion, and tests for
  invalid CSV, dates, amounts, filters and CLI failures. Codex 3 additionally split
  the CLI entry point from a testable main function. No actionable defect was
  confirmed in this focused review; passing these checks does not prove all behavior.
- Ten Linux harness tests passed, covering telemetry filtering, malformed input,
  partial JSONL, timestamp attribution, ambiguous event pairs, terminal shutdown,
  fixture materialization and guarded account-copy cleanup.
- Current-checkout Windows gates passed: `npm run typecheck`, `npm test` and
  `npm run check`. The suite reported 1,021 tests: 1,010 passed, 11 platform skips,
  zero failures. Coverage was 96.19% lines, 88.60% branches and 94.31% functions;
  package and installed-CLI checks passed with zero runtime dependencies.
  These gates ran after the live timings and are separate from the frozen Linux
  snapshot's 1,021-pass baseline. This experiment did not run a new CI matrix.
- Eleven temporary account files were removed after execution. Original account
  stores were untouched. Frozen inputs and private local evidence remain in the
  active laboratory; no raw session, credential, capture or generated project is
  added to the repository or runtime package.

## Confirmed interrupted run

Jecode 3 settled as failed after 274.156 seconds. Its seventh request failed with
`transportFailure: idle-timeout` after 137.118 seconds at the provider boundary.
It had received 11 application events and 17,918 characters; the first reasoning
event arrived at 14.016 seconds. No close code, size violation, tool error,
context compaction or clipped result was reported. All 13 issued tools had results.

The measured `ResponsesSession` limits each next-event wait to 120 seconds, inside
a separate 300-second model-progress budget. `SocketChannel` labels expiry of that
wait `idle-timeout`; the public failure normalizer renders the generic network
message. The observation proves that this local deadline terminated the request.
It does not prove that the upstream request would eventually have completed.

The partially completed project still passes the 56 external functional checks.
That is not full task completion: the requested tests/documentation and final
review were interrupted. Usage totals omit the failed response, whose final
usage never arrived. They must not be compared with completed-turn totals.

Before further speed tuning, investigate silence versus actual transport failure
with bounded sparse-event fixtures, cancellation and progress deadlines. Evaluate
any revised timeout policy on a new frozen snapshot, and keep this failed baseline.
Preserve the prohibition on replaying generation automatically after partial output.

The [progress-deadline follow-up](HEAD-TO-HEAD-PROGRESS-2026-09-07.md) records the
subsequent fix and a separate frozen comparison. The six outcomes above remain
the original baseline, including the failed attempt.

## Interpretation limits

Jecode's `providerMs` includes streaming through its adapter and synchronous stream
callbacks. Codex's corresponding approximation brackets serialized WebSocket send
and `response.completed` timestamps. Both contain network and provider time;
neither isolates inference. Codex send-call duration alone is not response latency.
Codex's turn TTFT and Jecode's first semantic event use different boundaries;
Codex also prewarms its connection during startup, which this task clock excludes.

Codex tool telemetry includes wrapper and nested operations. Model calls are counted
from canonical session items so wrapper timings are not added twice. Response usage
is summed per response and cross-checked against native cumulative session usage.
Input totals include repeated/cached context; they are not unique context size or
a billing estimate. Output totals are not a measure of useful code quality.

Three attempts per client on one task cannot establish a general speed ranking,
an incident rate, broad code quality or long-session reliability. This account-route
experiment does not measure Anthropic API, OpenAI API or Ollama API performance.
Add held-out debugging, larger repository and multi-turn/context tasks before
treating these measurements as evidence for daily use or the 1.0 release gate.
