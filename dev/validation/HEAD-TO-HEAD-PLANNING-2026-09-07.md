# Jecode planning experiment — 2026-09-07

Six uninterrupted live trials completed successfully. A development-only
instruction to group related edits reduced Jecode's observed completion time
on both new tasks, with all frozen acceptance checks passing. Codex was faster
on the planner. This is a screening result, not a general speed or quality ranking.
The production system prompt remains unchanged.

## Frozen comparison

- WSL2 Ubuntu, native Linux files and executables; same environment and pinned
  Codex CLI 0.153.4 as the [transport follow-up](HEAD-TO-HEAD-PROGRESS-2026-09-07.md).
  Native Node 22.23.2; same account, `gpt-6-astra`, high effort, one model-facing
  controller, no delegated agents, plugins, web or external task dependencies.
- Codex Fast mode is disabled with `service_tier = "default"`; Jecode leaves its
  optional service tier unset. Effective backend priority is not independently
  verified. Native prompts and tool interfaces differ by design.
- Jecode baseline: 473 source files, based on commit
  `768b686934184627cac3d7e30773f124405ae56f` plus the uncommitted transport work.
  Snapshot SHA-256:
  `3f26dd7f49a354e4341e0a5007cb6c6b3c3c29e3d005bb80cd3262d0c9d83cb1`.
- Grouped snapshot SHA-256:
  `e252d89c82d0524acae9ef431dc5c4aa3cf064397c06549c3040d0096e1b42f9`.
  Only `src/prompt.ts` differs. The variant adds the instruction quoted in
  [scenarios.py](../benchmarks/head-to-head/scenarios.py), preserving ordered
  writes, dependent-result inspection, tests and failure checks.
- The [declared sequence](../benchmarks/head-to-head/PLANNING-EXPERIMENT.json)
  was cache baseline/Codex/grouped, then planner grouped/Codex/baseline. Fresh
  workspaces and homes for every run; one live agent at a time. Each task has
  only one observation per configuration. Reversing order across tasks does
  not control all backend load, prompt-cache, generated-content or host effects.
- Both TUIs use a 140×40 PTY and `NO_COLOR`. Jecode uses reduced motion and its
  production permissions menu. Codex uses its native workspace-write sandbox;
  Jecode's approved shell is not OS-sandboxed. These are different boundaries.
- Timing starts at submission to an already-ready composer. Startup, sign-in,
  source checks and external scoring are excluded. Source checks finish before
  live timing. Light, read-only evidence inspection occurred during runs; this
  was not a dedicated quiet performance host.

Private evidence is retained under
`/var/tmp/jecode-head-to-head-20260907-planning-62d2`: source snapshots, harness,
run manifests, canonical sessions, numeric diagnostics, terminal captures,
original outcomes, external checks and verification reports. Do not publish raw
homes or captures. The [protocol](../benchmarks/head-to-head/PLANNING.md)
documents reproduction and promotion limits.

All six task runs and the separate recovery trial recorded the same WSL boot ID.
The first preparation attempt, retained under the sibling `planning-19c7`
laboratory, exposed scorer rejection handling and source-copy/string-generation
mistakes in the development harness. They were corrected before any live model
trial, then a new immutable laboratory passed preparation. Those failures are
not omitted model attempts and do not indicate a production Jecode regression.

## Tasks and outcomes

The cache task covers asynchronous generations, invalidation, exact expiry,
failures, and per-waiter cancellation. Its starting fixture passes 7/31 checks.
The planner covers graph validation, target closure, deterministic ordering,
durations, a 14,000-node chain and CLI errors across ten implementation modules.
Its starting fixture passes 8/57. It is a larger fixture, not a large production
repository. Both scorers were fixed before any live run and stayed outside the
agent workspaces.

| Task | Configuration | Completion | Frozen checks | Model requests | Output tokens |
| --- | --- | ---: | ---: | ---: | ---: |
| Cache | Jecode baseline | 342.924 s | 31/31 | 12 | 9,550 |
| Cache | Codex | 352.510 s | 31/31 | 11 | 9,987 |
| Cache | Jecode grouped | 323.807 s | 31/31 | 10 | 9,066 |
| Planner | Jecode grouped | 471.180 s | 57/57 | 10 | 13,971 |
| Planner | Codex | 382.624 s | 57/57 | 10 | 11,228 |
| Planner | Jecode baseline | 530.586 s | 57/57 | 12 | 15,792 |

Relative to Jecode baseline, grouped took 5.6% less time on cache and 11.2% less
on planner, with two fewer requests in each case. It also generated 5.1% and
11.5% fewer output tokens respectively. These observations do not isolate a
causal round-trip saving from variation in reasoning, generated content or
backend latency. Output usage includes the provider's reasoning accounting;
whole-task tokens divided by time are not decoder throughput.

Compared with Codex, grouped took 8.1% less time on cache and 23.1% more on
planner. No universal winner follows from these two tasks or a simple mean.

## Where the time went

| Jecode run | Provider adapter interval | Total outside that interval | Preparation included in outside time |
| --- | ---: | ---: | ---: |
| Cache baseline | 340.466 s | 2.458 s | 1.401 s |
| Cache grouped | 322.372 s | 1.435 s | 0.617 s |
| Planner baseline | 526.303 s | 4.283 s | 0.921 s |
| Planner grouped | 468.480 s | 2.700 s | 0.614 s |

The adapter interval includes network, upstream processing and local streaming
callbacks; it is not a pure inference measurement. Outside time also includes
tools, persistence and observation overhead. These measurements do not show a
large local inter-request bottleneck. They do not prove every operation inside
the adapter is upstream work.

All 44 Jecode requests completed over WebSocket, with 40 incremental/reused
requests, no fallbacks, compactions, clipped results, transport failures,
unmatched tool calls or dropped diagnostic records. The longest request was
238.201 seconds on planner grouped, reporting 7,796 output tokens and 40 received
events. Event totals lack per-event timestamps, so they cannot establish the
longest silent gap or prove an older deadline would have interrupted this run.

Codex's timestamped sends/completions yield unambiguous adapter-like intervals
of 350.945 s on cache and 379.692 s on planner. Their boundaries differ from
Jecode's. Usage is checked against the native session totals; untimestamped API
events and startup/prewarm events are not counted as task-generation requests.

## Planning and quality review

On cache, responses containing edits decreased from five to three (six to four
write calls). On planner, they decreased from five to three (twenty to eighteen
write calls). Baseline already batched edits: all five planner editing responses
contained multiple edits. The variant concentrated initial implementation across
eight files, then grouped tests, documentation and further fixes.

Adjacent edit-only responses are not automatically waste. Cache baseline first
produced a failing regression for cancellation during the injected clock's
cache-hit callback, then corrected its implementation and reran checks. This is
the one recorded Jecode tool failure in the uninterrupted trials; it was useful
verification, not a transport or harness failure. All three planner outputs use
an iterative graph algorithm and a minimum-priority queue.

The separate [cache probe](../benchmarks/head-to-head/explore-cache.mjs) was
created after observing that self-review. It examines callback reentrancy and
must remain a post-hoc observation; it does not replace or enlarge the original
31-check score. Both Jecode variants and the resumed Jecode output pass; Codex's
cache-hit path resolves instead of rejecting when the clock callback aborts the
caller. This narrow, output-derived case is not evidence of general superiority.
The injected clock remains finite and monotone, but its reentrant side effect is
more unusual than the original scorer's cancellation scenarios.

Independent reruns of the generated suites passed: cache baseline 30 tests,
cache grouped 32, cache Codex 33, planner baseline 154, planner grouped 116,
planner Codex 100, and resumed cache 35. None failed or skipped. These counts
reflect different parametrization and scope; they are not equivalent quality
scores. In particular, reducing generated verification content can affect both
time and breadth even when the common acceptance checks still pass.

## Interruption and resume

A seventh, separately declared run used baseline Jecode on a fresh cache
workspace. The driver waited for a persisted successful edit, sent Escape,
observed an interrupted checkpoint, exited the TUI and reopened it with `-c`.
It restored the ordinary session tool permissions before sending a continuation.

- Submission to interruption: 94.241 s.
- Escape to observed interrupted checkpoint: 125.120 ms.
- Interruption, process restart and ready-composer setup: 5.402 s total.
- Continuation to completion: 305.627 s.
- Whole recovery scenario: 405.271 s; external acceptance: 31/31.
- Before new input, workspace hashes and historical node hashes were unchanged,
  the footer was idle and no new completed/cancelled request diagnostic was
  recorded. This observation does not timestamp network request starts.
- The historical node stayed byte-identical after completion. The final two
  settlements are `interrupted` and `completed`; both decode with the measured
  production codec. All 35 project tests pass independently.

This verifies one graceful interruption at a persisted write boundary, process
restart and continuation. It is not a forced crash during a filesystem transaction,
an interrupted shell-tree test, a Codex recovery comparison, a compaction test
or release-candidate soak evidence. Do not pool its elapsed time with the six
uninterrupted runs.

## Verification and retained evidence

- All seven outputs pass the original and repeated external scorer. Sources,
  prompts, fixtures and scorer hashes remain unchanged for their recorded
  manifests. All have the one synthetic initial Git commit, no dependencies and
  no workspace symlinks; changed paths remain in source, tests, README and examples.
- Six Jecode node files decode with the frozen production session codec. The
  recovery hashes additionally establish preservation of its historical node.
- The frozen Linux baseline passes typecheck and all 1,029 tests, with no skips
  or failures. The grouped source passes typecheck. Only its prompt differs.
- Windows Node 24.18.0 `npm run check` passes: 1,029 tests, 1,018 passed and 11
  platform skips, no failures. Coverage: 96.20% lines, 88.65% branches, 94.26%
  functions. Package check: 192 files, 1,928,000 bytes, zero runtime dependencies;
  isolated installed CLI reports 0.8.7. No new GitHub CI run or release occurred.
- All 16 Linux harness tests pass, including scenario selection, isolated source
  variation, non-overwriting plans, interruption classification, ambiguous timing
  rejection, and guarded removal of account copies.
- Raw outcomes are preserved. Post-checks use separate filenames. The recovery
  driver and post-hoc probe were frozen separately; neither changes the six-run
  task inputs. Temporary account copies are removed after execution; original
  account stores remain untouched.

## Decision

Keep the grouped instruction as an experimental candidate. The direction is
promising on these tasks, but a production promotion needs additional held-out
work and repeated trials, as specified before the experiment. No reasoning-effort
reduction, skipped verification, tool scheduling change or production prompt
change was used to obtain the results.

The next useful experiments are repeated task pairs and a genuinely larger
repository, followed by a separately controlled instruction to use known task
paths for initial reads. The current three exploratory request stages can often
be observed directly; avoiding redundant discovery is a specific hypothesis to
test, not a reason to skip grounding reads. Preserve test coverage and inspect
whether verbose or duplicated output can be reduced without weakening checks.
