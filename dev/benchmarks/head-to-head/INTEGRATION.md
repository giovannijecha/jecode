# Integration comparison protocol — 2026-09-08

This repeat controls the subprocess-capture restriction observed in the
[quality follow-up](../../validation/HEAD-TO-HEAD-QUALITY-2026-09-08.md).
The private root is `/var/tmp/jecode-head-to-head-20260908-integrated-bc71`.

## Fixed before trials

- Native WSL, Node 22.23.2, pinned Codex 0.153.4, the same account,
  `gpt-6-astra`, high effort, no fast mode or requested paid priority.
- Codex retains its workspace filesystem sandbox. Its isolated named profile
  enables command network access, matching access available to Jecode commands.
  Filesystem enforcement still differs; no claim of identical sandboxes is made.
  Original personal settings are untouched. The profile must pass the offline
  pipe-capture probe and the unchanged starter's own suite before measurement.
- Keep the same planner-progress prompt, starter and 160-check evaluator from
  the previous follow-up. Freeze them again with their original hashes.
- Baseline contains current runtime corrections but the original grouping rule.
  Grouped differs only by the four-line instruction already tested. Do not
  reduce reasoning, scope, tests or failure checks. All writes remain ordered.
- Six new, serial, fresh-session trials: baseline/Codex/grouped, then
  grouped/Codex/baseline. Names: `integrated-baseline-1`, `integrated-codex-1`,
  `integrated-grouped-1`, `integrated-grouped-2`, `integrated-codex-2`,
  `integrated-baseline-2`. Keep one WSL parent alive for the batch.
- Run source gates before timing; no concurrent benchmark or heavy test load.
  Use the existing ready-composer clock and 1,200-second per-task deadline.
  Retain all failures; never average failure latency with completion latency.

## Decision and review

Adopt the grouped instruction only if both candidate turns complete, meet the
frozen external contract, preserve task scope and meaningful verification, and
source review finds no unresolved material regression. Compare elapsed time and
request counts with both baseline and Codex. Small-sample timing is descriptive;
a slower result must be reported, not hidden by an extra selected retry.

Rerun each output's ordinary tests outside the agent process. Review final code,
test changes, documentation and verification order; different generated test
counts are not quality scores. Reuse the independent planner probes as explicitly
supplementary checks. Check source/fixture/evaluator provenance and decode saved
Jecode nodes. Archive private diagnostics, then remove temporary account copies.

The offline transport work verifies disconnection handling and explicit recovery,
not the root cause of the previous upstream close code 1006. A clean batch cannot
prove this intermittent failure is eliminated. This experiment is not release
soak acceptance or evidence of general superiority over Codex.
