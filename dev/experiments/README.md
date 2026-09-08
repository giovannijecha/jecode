# Controller experiments

`work-state.ts` is an isolated daily-driver experiment. Production does not import
it or advertise its tool. The [comparison harness](../benchmarks/head-to-head/README.md)
can freeze it into a separate source variant with the normal permission registry
and canonical controller history. It introduces no second model worker.

The question is whether an optional, bounded checklist and observed command
results improve completion of complex work enough to justify their token and
request overhead. Simple turns are not required to create a plan. A checklist
does not prove correctness or authorize additional scope.

State is reconstructed only from matched, successful `work_state` results in
canonical history. Reading it never runs historical tools. It remains available
after model-context replacement or session resume without a new persisted schema.
Unmatched, denied, failed or invalid updates do not replace the last valid plan.
New user guidance is reported separately so earlier plans can be reviewed.

Recent command results are observed evidence, distinct from model-reported step
status. Later file-tool attempts mark earlier checks stale; later commands may
also modify files. This experiment cannot detect external edits or establish
that a passing command covers every requirement. It does not force another
model request after a final answer or repeatedly demand the same tests.

The [eighteen-trial evaluation](../validation/DAILY-DRIVER-2026-09-08.md) does
not support promotion: mean elapsed time increased 9.7% against current Jecode,
with more requests and weaker recovery in some generated outputs. Keep it
development-only. Offline continuity checks do not establish live benefits
after compaction; these trials did not compact.

A revised candidate needs new independent task checks, output/scope review and
repeated comparisons against the unchanged baseline. Do not add ceremony or
latency without a corresponding completion or quality benefit.
