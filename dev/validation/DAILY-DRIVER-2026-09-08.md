# Daily-driver investigation

## Decision

The eighteen declared trials completed, all original acceptance checks passed,
and independent reruns reproduced those results. Current Jecode is competitive
with Codex in this sample; neither establishes general output-quality superiority.

Do not promote the optional work-state tool. It increased mean elapsed time by
9.7% against current Jecode, used more requests, and produced weaker recovery in
some supplementary output checks. Keep it development-only. Keep the production
transport unchanged: the separate HTTP/WebSocket pilot does not establish a
latency advantage for either transport.

This phase adds failure-boundary coverage, an isolated experiment, and repeatable
evaluation/review tools. It introduces no new installed runtime feature. The
baseline already includes the previously integrated grouped-edit instruction
and pending provider work described in the
[integration report](HEAD-TO-HEAD-INTEGRATION-2026-09-08.md).
The earlier output-stage WebSocket 1006 remains unexplained; these passing runs
and additional diagnostics do not prove its cause has been fixed.

## Protocol and provenance

- Two new tasks: JSON configuration patching/migration and a streaming HTTP file
  server. Three configurations, three repetitions each, eighteen serial trials.
  The order was counterbalanced and fixed before timing. No failed or slow
  attempt was removed.
- Native WSL Ubuntu, Node 22.23.2, pinned native Codex CLI 0.153.4, the same account,
  model `gpt-6-astra`, high effort, and configured default tier with Fast off.
  Backend routing equivalence is not independently observable.
- Both clients used native interactive interfaces and their own tools and
  permission mechanisms. Same fixtures, literal task prompts, and host; different
  harness prompts and security implementations remain part of the comparison.
- Baseline commit: `768b686934184627cac3d7e30773f124405ae56f`, with the frozen
  working-tree changes. Snapshot SHA-256:
  `26c57e13de3a37e221e7d9a4aeb4b822e6a7fe8645b102549921bec553aa6a6c`.
  A commit alone does not identify this development build.
- Private laboratory: `/var/tmp/jecode-head-to-head-20260908-daily-f294`.
  Historical inputs, original scores, and raw evidence remain private and unchanged.
  The rejected `daily-f293` preparation ran no timed model trial: candidate
  registry expectations still counted seven tools. The new freeze adjusts three
  candidate-only expectations to eight without relaxing permission checks.
- Original evaluators were calibrated against reference implementations and four
  deliberate mutations per task, before any timed trial. Configuration starts at
  12/53 checks and its reference passes 53/53; server starts at 13/57 and its
  reference passes 57/57. Calibration ran natively and in the selected Codex
  sandbox; it is Linux-only because this Windows environment cannot create the
  required file symlink.
- All eighteen output reviews were recorded under randomized aliases before
  opening identity maps. Light source review overlapped later serial trials;
  heavy reruns and supplementary probes waited until timing finished. Review
  feedback never reached participants, and frozen inputs were not edited.

The finite acceptance suite is not a complete specification oracle. References
also have gaps exposed by supplementary probes; they are calibration tools, not
authoritative implementations. Original scores are retained separately from
post-hoc findings.

## Task results

Mean elapsed time and observed range, in seconds; three repetitions per cell:

| Task | Current Jecode | Work-state experiment | Codex |
| --- | ---: | ---: | ---: |
| Configuration migration | 593.945 (585.815–598.648) | 640.046 (611.502–686.335) | 613.296 (456.921–760.693) |
| HTTP file server | 599.991 (569.188–640.174) | 669.523 (617.567–705.339) | 606.605 (535.011–747.292) |
| Mean across both tasks | 596.968 | 654.784 | 609.951 |

Current Jecode's mean elapsed time is 3.2% lower on configuration work and 1.1%
lower on server work than Codex, or 2.1% across both. These are descriptive
differences in a small, variable sample, not a statistically established speed
advantage. Codex's final server trial includes an automatically recovered error;
its entire duration remains in the comparison.

Every configuration completed all six tasks without human intervention and
passed all original checks: 53/53 per configuration task and 57/57 per server
task, totaling 990/990 repeated assertions across eighteen outputs. Independent
acceptance reruns and each output's own test suite also passed. Test counts are
not measures of equal coverage or general accuracy.

Original trials, in their fixed execution order:

| Run | Elapsed seconds | Original acceptance |
| --- | ---: | ---: |
| config-baseline-1 | 598.648 | 53/53 |
| config-work-1 | 611.502 | 53/53 |
| config-codex-1 | 456.921 | 53/53 |
| server-work-1 | 617.567 | 57/57 |
| server-codex-1 | 537.513 | 57/57 |
| server-baseline-1 | 569.188 | 57/57 |
| config-work-2 | 622.300 | 53/53 |
| config-codex-2 | 760.693 | 53/53 |
| config-baseline-2 | 597.373 | 53/53 |
| server-codex-2 | 535.011 | 57/57 |
| server-baseline-2 | 640.174 | 57/57 |
| server-work-2 | 685.663 | 57/57 |
| config-codex-3 | 622.275 | 53/53 |
| config-baseline-3 | 585.815 | 53/53 |
| config-work-3 | 686.335 | 53/53 |
| server-baseline-3 | 590.611 | 57/57 |
| server-work-3 | 705.339 | 57/57 |
| server-codex-3 | 747.292 | 57/57 |

## Where the time goes

| Configuration | Completed responses with usage | Reported output tokens |
| --- | ---: | ---: |
| Current Jecode | 71 | 106,410 |
| Work-state experiment | 91 | 112,534 |
| Codex | 64 | 108,688 |

Codex additionally has one error-bearing attempt without reported usage, so its
task send count is 65. Tokens for that attempt are unavailable; the table is not
a complete cost comparison. Codex's native execution wrapper can contain nested
operations, so raw model-facing tool counts are not directly comparable.

The experiment adds 28.2% more completed requests and 5.8% more reported output
tokens than current Jecode. It invokes `work_state` nineteen times across six
tasks. The observed result does not justify adding the tool to ordinary turns.

For current Jecode, recorded provider-adapter intervals sum to 3,562.068 seconds
out of 3,581.809 seconds elapsed, over 99%. This includes transport, model
generation and stream handling; it is not pure server inference time.
Preparation sums to 5.105 seconds across six tasks, 0.625–1.164 seconds per task.
Typical preparation is a few milliseconds per request after initial metadata.
Tools, preparation, observation and other local work occupy the remaining wall
time. These trials do not identify a material local processing bottleneck.

All 162 requests across both Jecode configurations completed. The 150
continuations reused WebSocket connections with incremental input. There were
no observed fallbacks, clipped results, compactions, unmatched tool calls, or
dropped recorder events. Three intermediate test commands failed during
development and were corrected before completion; these were tool results,
not transport failures.

Grouped edits are already common. Some adjacent edit-only responses remain,
but their duration includes generating necessary code. The batching report
cannot establish that all such intervals are removable overhead. Blindly
combining dependent changes would not be a supported optimization.

## Output review and supplementary checks

All eighteen outputs preserve the requested exports and original smoke tests,
introduce no dependencies or unrelated product features, and make no commits.
Implementation responsibilities are generally separated into small modules.
Independent own-suite counts agree with final-answer test counts, and every
workspace passes `git diff --check`. Extra reported checks, including a 64 MiB
stream hash, repeated suites and timezone cases, are supported by saved commands
or test bodies. Passing tests still leave untested behavior.

The same post-hoc probes ran against every output of the relevant task, after
source review and timing. The table reports outputs passing each probe, out of
three; it does not replace the original scores or produce an overall ranking.

| Supplementary behavior | Current Jecode | Work-state experiment | Codex |
| --- | ---: | ---: | ---: |
| Restore prior files when subsequent writes fail but rename remains available | 3/3 | 1/3 | 3/3 |
| Conditional ETag lists tolerate leading/interior empty elements | 1/3 | 2/3 | 2/3 |
| Premature EOF before the first byte yields a bounded 500 and closes the file | 3/3 | 2/3 | 3/3 |
| Invalid non-HTTP conditional dates are ignored | 3/3 | 3/3 | 3/3 |

The migration outputs `config-work-1` and `config-work-3` create rollback backups
only after a failure. Under persistent write failure they cannot restore earlier
committed files, whereas implementations that stage backups before committing
recover through rename. The injection covers promise-based writeFile and
FileHandle.writeFile while rename remains available; it does not simulate every
filesystem failure.

The server outputs `server-baseline-1`, `server-baseline-2`, `server-work-2`,
and `server-codex-1` miss empty-element tolerance in conditional ETag lists.
HTTP recipients must tolerate reasonable empty list elements.
[RFC 9110, list recipients](https://www.rfc-editor.org/rfc/rfc9110.html#section-5.6.1.2)

`server-work-3` closes its handle but resets the connection on premature EOF
instead of producing the tested bounded 500 before headers. The injected read
was exercised exactly once in all nine server outputs; all closed their handles.
Full-GET, valid ETag, wildcard and opaque-comma controls passed. Invalid date
expectations follow the conditional request rules.
[RFC 9110, If-Modified-Since](https://www.rfc-editor.org/rfc/rfc9110.html#section-13.1.3)

Malformed tag prefixes, suffixes and embedded wildcards are descriptive only:
HTTP permits recovery from invalid constructs, so those observations are not
graded as defects.
[RFC 9110, error handling](https://www.rfc-editor.org/rfc/rfc9110.html#section-2.4)

Explicit file-open counts at 4/16 migration entries range from linear 16/64 to
32/320 across outputs. Some extra reads revalidate already committed files to
detect concurrent changes. This is a resilience/performance tradeoff, not proof
of waste; the instrumentation does not count all kernel I/O or measure elapsed
performance. The reference configuration also fails the compound-write probe,
and the reference server accepts non-HTTP dates. Their original acceptance
scores were not changed or used to dismiss participant findings.

The work-state candidate therefore has neither a measured speed benefit nor a
convincing quality benefit. Current Jecode and Codex both have output gaps.
These are generated-artifact findings, not identified defects in Jecode's own
HTTP implementation, and do not justify task-specific hints in the core prompt.

## Transport and recovery

In `server-codex-3`, a request sent at 14:46:05.184 UTC ends at 14:47:14.257
with an error-bearing completion and no usage. A new WebSocket connection
succeeds at 14:47:15.042 and another request follows at 14:47:15.045. The task
finishes without intervention. Error text was deliberately not captured, so its
cause is unknown; this is not evidence of the same failure as Jecode's earlier
1006. The full 747.292 seconds stays in the result.

The current analyzer now counts the presence of error attributes separately from
collector parse errors and successful usage events. This post-timing analysis
change preserves the frozen collector, raw events and outcomes; it does not
invent missing messages or usage. The saved rollout contains no corresponding
stream-error event, so rollout-only inspection would miss this observation.

Six separate serial transport probes each completed two requests:

| Transport | Mean two-request duration | Mean output tokens | Mean second-request bytes |
| --- | ---: | ---: | ---: |
| WebSocket | 113.486 s | 3,073 | 827 |
| Forced HTTP | 99.396 s | 2,862.7 | 16,531.3 |

All WebSocket second requests reused a valid incremental prefix. HTTP completed
sooner in this tiny sample while generating fewer tokens; that does not establish
transport causality. Smaller WebSocket wire payloads do not imply smaller logical
context or lower billed tokens. The forced-HTTP variant is a development probe,
not a new setting or a failed WebSocket upgrade. No transport switch is promoted.

## Verification and evidence lifecycle

- Sixteen new loopback failure cases cover HTTP and WebSocket before acceptance,
  after acceptance, after text and after a completed tool item, with and without
  a prior committed effect. They verify no partial-tool execution, duplicate
  effects, ambiguous replay or silent fallback, and explicit continuation with
  retained history. All pass on Windows and WSL.
- Seven focused work-state checks pass, including cancellation, invalid/unmatched
  updates, stale evidence, guidance, resume and context replacement. The seventh,
  added after the live freeze, verifies the real durable store/codec and a
  persisted compaction anchor. No persisted schema or production import is added.
- Frozen baseline and candidate passed WSL typecheck and all 1,059 tests before
  timing. Forced HTTP passed typecheck and 27 focused API/HTTP tests.
- Final Windows `npm run check` passes: 1,060 tests, 1,049 passed, eleven
  platform skips, zero failures/cancellations. Coverage: 96.22% lines, 88.81%
  branches, 94.27% functions. Package: 192 files, 1,929,271 unpacked bytes,
  zero runtime dependencies; installed CLI validation passes at version 0.8.7.
- All 32 Python harness tests pass after the analyzer update. Supplementary
  probes completed for all eighteen masked outputs without changing their bytes.
  The runner's zero exit status means probe integrity; individual failures remain
  recorded above.
- Independent verification confirms source, prompt, fixture and evaluator hashes;
  original and own-suite reruns pass. Twelve Jecode saved nodes validate through
  the frozen production codec. All original and masked workspace hashes still
  match the review snapshots.
- Private evidence includes outcomes, acceptance/rechecks, request observations,
  rollouts, masked review notes, identity mappings and supplementary results.
  Twenty-two temporary account copies in the measured laboratory and two in the
  rejected preparation were removed. Production accounts were not modified.
  Windows verification logs were retained in the private laboratory. Automatic
  approval review rejected removal of the owned
  `%TEMP%/jecode-daily-f294-verification` directory with `blocked by policy`;
  its two log files remain. No alternative deletion route was attempted.

This is development evidence, not the release-candidate soak record. No commit,
PR, merge or release was performed as part of this phase.

## Next decision boundary

Keep the current single-controller architecture, bounded tools, ordered effects,
explicit permissions and canonical history. Avoid mandatory planning or generic
review loops that consume requests without demonstrating better completion.

The next useful experiment is contract-focused verification on new, longer
repository tasks: changing requirements mid-turn, failure recovery, resumed
sessions and actual compaction. Freeze acceptance before execution, review
requirements beyond happy paths, measure time to a verified result, and retain
failed attempts. Do not tune prompts to the exact headers or write faults above.

Safe automatic recovery deserves a separate, narrowly scoped design and fault
experiment. A reconnect is not authorization to replay a partially completed
generation or tool effect. The earlier 1006 and Codex's observed recovery identify
a useful comparison question, not a ready-made retry policy.

This sample covers two new bounded Node tasks with one model/account route.
It does not establish superiority on long multi-session work, every task family,
all providers, real terminal accessibility, or production soak reliability.
