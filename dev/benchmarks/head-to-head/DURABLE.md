# Durable execution comparison — 2026-09-08

This extends the integration comparison to a new, larger change across planning,
asynchronous scheduling, cancellation, persistence, recovery and documentation.
The initial implementation is the same frozen planner starter used in the prior
integration experiment, not a selected new participant output. Its existing
public API and 57 original external checks remain requirements.

The new task and external evaluator are frozen before any model sees the task.
Validate the evaluator against an independent reference and intentional defective
variants first. The reference and evaluator stay outside participant workspaces.
Every participant gets the complete written contract and the same starter.

## Variants and decision

- Baseline Jecode includes the previously integrated edit-grouping instruction.
- The experimental `contract` variant adds a general instruction to inspect
  callers, tests and documented behavior, preserve compatibility, inspect
  cancellation/recovery boundaries, and investigate rather than weaken failures.
  It supplies no task-specific algorithm, hidden test input or extra model worker.
- Codex retains its native instructions and tools. All use the same account,
  Astra/high, native WSL Node 22.23.2, Codex 0.153.4, and the previously verified
  network-enabled workspace permission profile. Backend priority remains unverified.
- Run six fresh, serial trials: baseline/Codex/contract, then contract/Codex/baseline.
  The deadline is 1,800 seconds per task. Keep setup failures, incomplete turns
  and incorrect completed outputs visible; never average failure latency as a win.
- Freeze runtime snapshots, fixture, prompt, evaluator and harness. No changes
  to those inputs during the batch. No heavy concurrent tests during timing.

Adopt the contract instruction only with no observed material correctness or
scope regression, preserved meaningful verification, and a useful measured
improvement in quality or execution. Faster incorrect output cannot qualify.
If results are mixed, keep the production prompt and report the experiment.
Fix a confirmed runtime defect independently when it has a reproducible regression.

## Interpretation and review

Compare completion, contract checks, seeded schedules, cancellation quiescence,
durable restart behavior, code/test review, requests and elapsed time. Tests derive
from the written contract rather than a participant implementation. Checks added
after seeing output must be reported separately. Preserve original artifacts.
Use deterministic gates instead of sleep-based race assertions where possible.
Rerun generated tests independently and check the old planner contract.

This is a substantially more complex task, but duration, actual context pressure
and compactions must be observed rather than manufactured or assumed. A task that
finishes in minutes does not establish hour-long session reliability. Two trials
per variant on one larger task cannot establish superiority across coding work.
Finish with a sanitized report, exact provenance and temporary credential cleanup.
