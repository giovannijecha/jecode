# Output quality review

## Decision and criteria

Evaluate the saved cache and planner outputs before promoting the grouped prompt.
Freeze this rubric before detailed source review. Correctness and completion are
prerequisites; elapsed time is considered only alongside those outcomes.

| Dimension | Evidence required |
| --- | --- |
| Correctness | Original external checks, additional contract-derived probes, reproducible defects with inputs and expected behavior. |
| Scope and compatibility | Requested exports and behavior preserved; no weakened existing tests, unrelated features, dependencies or execution of planned commands. |
| Maintainability | Cohesive responsibilities, understandable state/ownership, duplication with a concrete maintenance consequence; no score from line counts alone. |
| Verification | Deterministic tests of failures and races; assertions on observable behavior; useful red/green cycles distinguished from infrastructure errors. |
| Execution | Reads supporting edits, result inspection before dependent changes, verification after the final relevant change, accurate final claims. |
| Changeability | A separately specified follow-up change, frozen acceptance checks and preservation of the original contract. Report unmeasured criteria as such. |

Use findings with severity and evidence rather than a subjective overall number.
Classify each dimension as `verified`, `finding`, or `not established`; a common
acceptance pass does not establish maintainability or complete correctness.
Retain failed attempts and original outcomes. Supplementary checks are explicitly
post-hoc and never change the original scorer or completion times.
The verifier reports the original run settlement and fails its aggregate gate
for failed/interrupted turns even when their partial artifacts pass every check.

## Identity masking and limits

`quality.py prepare LAB DESTINATION` copies only the six completed uninterrupted
workspaces into randomized per-task labels, excluding Git metadata and all homes,
transcripts, timings and client names. It records the source hashes separately
and refuses an existing destination or symbolic links. Review copies are local
and disposable; the original outputs are never edited.

This is identity masking, not a fully blind independent review: the reviewer has
seen prior results and a narrow output-derived cache finding. Do not claim that
masking removes that prior knowledge. Record artifact review before unmasking;
then examine process evidence without confusing intentional failing regressions
with agent failures. New probes must apply identically to all outputs.

## Work sequence

1. Freeze the rubric and output provenance; review all six implementations.
2. Add independent, deterministic probes from the written task contracts. Keep
   discovered edge cases and implementation-inspired checks separately labeled.
3. Trace confirmed issues to Jecode's instructions, tools or controller where
   possible. A generated-code defect is not automatically a runtime defect.
4. Implement the smallest justified change, with reproducible regression checks.
   Keep experimental prompts out of production until fresh evidence supports them.
5. Recheck source/package gates and publish a sanitized local validation report
   covering findings, counterevidence, changes and remaining measurement gaps.

No comparative quality claim is based on generated test counts, token counts or
one favorable example. Larger repositories, extended context and independent
review remain separate evidence requirements.

## Follow-up experiment

The [declared six-run sequence](QUALITY-EXPERIMENT.json) repeats one new planner
change twice per configuration in reverse order. `followup.py LAB REVIEW`
materializes a common starter under the frozen Linux harness: the planner output
with the smallest SHA-256 of its complete file manifest, selected without using
client identity, correctness findings or elapsed time. All six runs receive
exactly the same files, task and external checks in fresh sessions. This measures
extension of existing code; it does not compare continuing each agent's own
conversation or measure the maintainability of three different starting artifacts.

The change introduces completed-task cut points, preserving full-graph validation
and the original planner contract. The new scorer runs all original 57 checks
plus new examples, generated DAGs, frozen inputs and CLI checks. Freeze the new
scorer and verify that the working starter fails the extension before live trials.
The CLI control-character requirement is now explicit for every participant;
it is not an undisclosed edge case. Original-output probes remain post-hoc.

Use the same model/effort, account and native clients as the previous experiment.
Record the `read_file` result correction in both Jecode snapshots. Only the
existing grouped instruction differs between Jecode variants. This is a small
repeated experiment, not proof of a general speed or quality advantage. Retain
the production prompt when results are mixed or have unexplained quality failures.

```sh
python3 quality.py prepare PREVIOUS_LAB REVIEW
python3 quality.py cross REVIEW --node NODE
node quality-probes.mjs cache REVIEW/outputs/cache-1
python3 followup.py LAB REVIEW
python3 batch.py --root LAB --prefix quality --plan QUALITY-EXPERIMENT.json
```

Cross-testing uses the test author's examples/documentation and the other
participant's `src/`. Wording-sensitive assertions and differences beyond the
written contract need manual triage; raw matrix failures are not product scores.
Harness verification includes `python3 -m unittest discover -s . -p 'test_*.py'`.
