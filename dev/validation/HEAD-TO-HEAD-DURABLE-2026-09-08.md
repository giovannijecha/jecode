# Durable execution and output quality — 2026-09-08

## Protocol and source

This follows the [integration repeat](HEAD-TO-HEAD-INTEGRATION-2026-09-08.md)
with the [durable execution protocol](../benchmarks/head-to-head/DURABLE.md).
The private evidence root is `/var/tmp/jecode-head-to-head-20260908-durable-a61f`.
Six new serial trials compare current grouped Jecode, an experimental contract
instruction, and native Codex on one new stateful implementation task. Each has
a 1,800-second deadline and a fresh session/workspace. The order is
baseline/Codex/contract, then contract/Codex/baseline.

The existing 16-file planner starter is unchanged, including its 116 own tests.
Its installed-name manifest hash remains
`11661ff6ba4a0141f887f79bbd830d391f45512bfa5fb65f34695158064ba3a2`.
The task adds dependency scheduling, bounded concurrency, asynchronous commit
barriers, cancellation quiescence, restart, checkpoint validation and atomic file
storage without changing the read-only CLI or adding dependencies. Every client
receives the full written contract. No reference implementation or evaluator is
placed inside participant workspaces.

The current Jecode baseline has 504 frozen source files, SHA-256
`15f7829c26c6093aea8035d87b1458b57952122a38d6a66abdac335e0eb28d24`, based on
`768b686` plus local changes. The contract variant differs only in `src/prompt.ts`,
with snapshot SHA-256
`6d92b3f0d3e8a8fb39c561fb4272a3883a8323b8973b62de063bff6b2901370f`.
The execution harness is hashed separately, including setup corrections made
after source freezing; its files are not changed during the live batch.

Native WSL Node 22.23.2, Codex CLI 0.153.4, the same account, Astra/high and the
previous network-enabled workspace profile are retained. Fast mode is off and
no paid priority is requested; actual backend routing is not independently known.
Filesystem permission mechanisms remain different between products.

## Evaluator and preparation audit

Before live requests, 78 new contract checks pass against a separate reference;
57 old planner checks pass on the unchanged starter. The new exports are absent
from the starter, so its aggregate score is 57/135. Four deliberate mutations
(serializing workers, disabling persistence, hiding failure status and doing no
work) are all detected. These are evaluator sanity checks, not proof that the
evaluator covers every defect.

The first materialization check compared installed filenames with `.template`
fixture filenames and rejected the matching starter. It was corrected to compare
the original installed-name manifest, without changing any fixture bytes. Both
TUI preflights subsequently reached the expected model and composer.

An additional sandbox probe command omitted the native Node directory from PATH
and failed before launching its checks. The shell sequence did not stop before
starting the first baseline. The corrected three-mode stdio probe passed after
that trial began (about 0.2 seconds of local probe work); the intended new sandbox
starter-suite rerun was not performed before timing. The identical starter/profile
passed in the prior integration laboratory. This is a preparation deviation,
not an agent failure, and the first baseline is retained rather than silently
replaced. No heavy test suite runs alongside live tasks.

The checkout now supports `batch.py --precheck`: preparation failure prevents
timed-run declaration/launch. The preparation command uses native Node in PATH,
checks the plan's task/variant set and selected sandbox, and subprocess probes
return failure for capture errors even with child status zero. This correction
applies to future batches; it does not retroactively erase the deviation above.

While the first trial was still in progress, before any scored participant
output was inspected, review found that one evaluator callback tried to mutate
the worker task unconditionally. The contract permits a frozen worker copy.
The corrected callback mutates only an unfrozen deps array; checkpoint isolation
is still checked. The frozen live evaluator is retained. A separate corrected
evaluator was applied to every saved output, with both original and reviewed
scores reported. No participant artifact or measured time is modified.

New manifests hash all evaluator modules, not just their entrypoint; this closes
a provenance gap exposed by splitting the new checks across modules. Legacy
manifests keep their explicitly reported entrypoint-only scope.

One original check requires rejection of reordered task records in a loaded
checkpoint. The written task requires canonical snapshots, but also calls
reordering tasks/dependencies equivalent without explicitly restricting that
equivalence to caller definitions. Treat this as an interpretation-sensitive
format check, not evidence of lost work or broken restart. Keep its raw result;
do not use this single point to claim a functional quality advantage.

Source review also motivates separate probes, applied uniformly after timing:
non-Error adapter/worker failures, stable and changing accessor inputs, and
Promise-resource scaling with one slow worker and many short independent tasks.
These checks were prepared after inspecting initial participant output and are
not blind or part of the predeclared 135-check score. The resource probe counts
`init` events without a matching `promiseResolve` event;
[Node documents](https://nodejs.org/docs/latest-v22.x/api/async_hooks.html#promiseresolveasyncid)
that resolution is not necessarily fulfillment. It does not measure retained
heap bytes or uninstrumented latency.

## Validation and results

Before timing, Windows `npm run check` passes. The contract candidate passes
native WSL typecheck and all 1,037 tests with no skips. The harness passes 24
offline tests. This clean Windows run does not establish the cause of the
previous unreproduced test-worker exit.

All six declared trials have settled. No interrupted time is included in a
successful-task average. Original and corrected evaluator scores are identical:

| Trial | Settlement | Elapsed | Original / reviewed checks | Own suite, independently rerun | Additional failure checks |
| --- | --- | ---: | ---: | ---: | ---: |
| Jecode baseline 1 | Completed | 11m22.161s | 134/135 | 191/191 | 38/38 |
| Codex 1 | Completed | 11m38.594s | 134/135 | 142/142 | 38/38 |
| Jecode contract 1 | Completed | 11m12.559s | 134/135 | 142/142 | 36/38 |
| Jecode contract 2 | Completed | 12m38.836s | 134/135 | 141/141 | 38/38 |
| Codex 2 | Completed | 10m22.922s | 134/135 | 142/142 | 36/38 |
| Jecode baseline 2 | **Failed stream** | **5m19.870s to failure** | 134/135, partial artifact | 116/116, starter tests only | 36/38, partial artifact |

The shared missed point is the interpretation-sensitive ordering check described
above. Neither score makes the failed baseline a completed task: it had not added
its regression tests, verified the change or produced a final response. The strict
batch verifier returns failure; the session codec check passes. All original
workspace files remain byte-identical before and after independent verification.

Codex completes 2/2 at a mean 11m00.758s. The contract variant completes 2/2 at a
mean 11m55.697s, 8.3% longer in this sample. Current Jecode completes 1/2, so its
single successful 11m22.161s does not support a balanced speed comparison. Two
trials per configuration cannot estimate a reliable general completion rate.
More generated tests are not inherently evidence of higher quality.

## Output review

The additional failure checks exercise thrown/rejected non-Error values during
load, initial save, commit and worker failure, plus stable/changing accessor
inputs. Every output passes the non-Error and stable-accessor cases. Contract 1,
Codex 2 and the unfinished baseline 2 allow an invalid captured task id/duration
to reach a worker or checkpoint callback when a getter changes between validation
and copying. Their capture paths do not revalidate the detached values. Baseline
1, Codex 1 and contract 2 pass both cases. This is an API-boundary hardening
finding, not evidence that ordinary JSON input failed.

The Promise probe also exposes different long-running scheduler behavior:

| Artifact | Resources without resolution hook at 1,000 short tasks | At 5,000 |
| --- | ---: | ---: |
| Baseline 1 | 10 | 10 |
| Codex 1 | 1,011 | 5,011 |
| Contract 1 | 9 | 9 |
| Contract 2 | 1,011 | 5,011 |
| Codex 2 | 11 | 11 |
| Baseline 2, unfinished | 1,010 | 5,010 |

Every artifact refills the free slot while the slow worker remains pending, and
every count returns to one after release. The linear cases repeatedly race an
active collection containing the same slow Promise; the bounded cases use worker
completion notifications. These are transient scaling differences in generated
code, not proven retained-heap leaks or Jecode runtime defects. Jecode's own
controller already uses a worker pool rather than this repeated-race pattern.

Source modules remain small (largest new source module 140-164 lines across
these artifacts); all original planner tests remain present. The current
baseline's first output is strong, but the other results prevent a claim that
either client consistently produces the better implementation.

**Decision:** retain the existing grouped production prompt. The extra contract
instruction does not deliver a consistent quality improvement and is slower than
Codex in this sample. It remains an isolated experiment, not a runtime change.

## Local overhead and the broken stream

Baseline 1 makes 13 requests, with 12 socket/prefix reuses, no fallback, no
compaction, no clipped tool results, 42 matched tool calls/results and no tool
errors. Preparation totals 1,102 ms, including 1,062 ms on its first request;
median preparation is 3 ms. Provider send intervals total 675,744 ms of the
682,161 ms turn. These intervals include networking, model work and local stream
consumption; they do not measure server-only computation. The difference also
includes tools, persistence and rendering. There is no large measured local
preparation bottleneck in this run. Reported input usage across requests is
269,071 tokens, of which 223,616 (83.1%) are cached; this cumulative usage is not
the simultaneously occupied context window.

The other Jecode trials also show zero compactions/clipped results. This batch
therefore does **not** validate operation under repeated context compaction.

Baseline 2 completes five requests and 20 tools before request six closes with
WebSocket code 1006 at `responseStage: output`. That request lasts 124,573 ms,
with first visible thinking at 12,264 ms, 15 received messages and 19,602 decoded
characters. It is below the five-minute progress deadline and decoded-event
limits. Input is 15,806 tokens against a 232,560-token request limit. There is
no recorded native network code. These data do not establish whether the cause
was the provider, an intermediary, the network or native frame processing.

No incomplete response tool is executed, no committed result is missing, and
there is no automatic replay. Adding a blind retry here would change the public
recovery contract and obscure the original failure.

The checkout now records connection age, age of the last decoded message, TCP
EOF and native-error observation before local teardown. Tests cover an unframed
EOF, a reserved frame opcode, local event/JSON rejection and resetting per-request
message age on a reused connection. They pass on Node 22/WSL and Node 24/Windows;
the native-error flag is intentionally not treated as a cause classifier because
Node versions can emit it for EOF too. This improves diagnosis; it does not prove
that the observed 1006 failure has been eliminated.

## Separate recovery and final gates

A separate continuation imports the verified failed conversation into a copied
workspace using the runtime store, with unchanged node content. Original trial
artifacts and timing stay untouched. Its diagnostic-only runtime snapshot is
`ef822a5f5c54ee34a0dd9461dc3e6364a63ba3a22c048fe29acc4a7c98dcb38c`;
the five changed files contain diagnostics/types and their regression checks.
The continuation completes in **10m46.927s**, using 13 successful requests,
12 socket/prefix reuses and 48 new tool calls. There are no tool errors, missing
results, compactions, clipped results, transport failures or dropped diagnostic
records. No request or workspace/history change occurs before the new message.
The copied historical node remains unchanged after completion; the original
failed trial remains byte-identical. Six stored nodes pass the production codec.

The recovered artifact passes 143/143 own tests and 134/135 original/reviewed
external checks, with the same ordering ambiguity. It still fails the two
changing-accessor cases (36/38 supplementary checks). Its scheduler now stays at
nine unresolved-hook resources in both queue probes, improving that aspect of
the partial implementation during its own review. External verification leaves
all artifact files unchanged. The evaluator/probes were never supplied to the
model as feedback.

The measured failure plus continuation totals **16m06.797s**, excluding the
offline investigation, copying and restart/setup interval between them. This
is a lower bound on recovery-inclusive user time, not a replacement benchmark
latency. A successful continuation establishes this saved-work recovery case;
it does not establish the cause or elimination of the original disconnection,
nor does it make the remaining output defects acceptable.

The strengthened preparation command passes all eight gates and 25 offline
harness tests after timing, including 1,037 runtime tests for both frozen Jecode
variants, reference/mutation checks and all 116 starter tests in native Codex's
selected sandbox. The first review-harness copy omitted two historical fixture
directories and failed two harness tests; those setup results are retained and
the complete preparation command was repeated after restoring the unchanged
fixtures. Neither preparation run is part of the timed comparison.

After the diagnostic change, Windows `npm run check` passes, including types,
coverage, package and installed CLI checks. Native WSL `npm test` passes all
1,037 tests with no failures/skips. The final harness passes 26 tests, including
the additional guard that a separate recovery copy cannot satisfy a missing
declared trial. The original comparison remains six trials, including its failure.

Eleven temporary account files are removed after all trials and the continuation
settle. Production account stores are untouched. Private traces and immutable
artifacts are retained for review; no credentials, generated participant output,
or raw transcripts are added to the repository. No release or PR is performed.
Fifteen owned local verification logs/helpers were archived with matching
SHA-256 hashes. Automatic approval review rejected their subsequent deletion
with `blocked by policy`; these files remain local and ignored by Git.

## Next decision

The evidence supports retaining the earlier batching optimization and the new
diagnostics/preparation guards. It does not support promoting the contract
instruction or claiming general superiority to Codex. The next product target
is dependable stream continuity and recovery, followed by output verification
that catches state-boundary defects consistently. Resolve the ambiguous ordering
requirement before reusing this task in a new declared experiment; preserve this
historical score rather than silently editing it.

A further comparison needs held-out multi-turn work with observed compaction and
long-lived state, plus interruption/restart scenarios on both clients. Repeating
this one task or merely extending its timeout would not supply that evidence.

This experiment is not release-candidate soak acceptance. A difficult task that
finishes in minutes does not prove reliability in hour-long, multi-turn sessions.
