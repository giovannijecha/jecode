# Daily-driver output review

Apply this rubric to the identity-masked outputs before opening their client map.
The timed task fixtures and acceptance evaluators are frozen independently.
Generated test counts and source length are descriptive, not quality scores.

## Review questions

1. Does the implementation meet the literal task and preserve the initial API
   and smoke tests? Record each concrete missing behavior separately from an
   ambiguous or additional review expectation.
2. Is the change limited to the requested work? Flag unrelated product surface,
   dependencies, publication, weakened tests, or new requirements invented by
   the agent. Additional internal validation is not automatically scope creep.
3. Are ownership, commit boundaries and failure handling understandable across
   the modules? Small files alone are not sufficient evidence. Look for duplicated
   validation and state that can disagree rather than imposing a preferred style.
4. Are resources bounded and released? Inspect file stream cancellation, HTTP
   error paths, staging cleanup, and rollback after partial progress. Distinguish
   a demonstrated defect from a race or resource concern needing a probe.
5. Does verification exercise the requirements and meaningful failure cases?
   Does the final answer distinguish observed checks from unperformed checks and
   avoid claiming broader completion than the artifact supports?

For configuration migration, trace validation before writes, each commit and
rollback boundary, cancellation after the final callback, preservation of BOM
and permissions, and what happens if a target changes during the operation.
Distinguish the literal precommit check from stronger concurrent-writer guarantees
that the prompt did not request. Check input detachment and unsafe keys without
inventing additional support for non-JSON objects.

For the server, trace the file handle from open through disconnect or error,
including failures before response headers have been sent. Check that error
status, body and Content-Length agree, that HEAD stays bodyless, and that date
and range parsing follows the requested contract. A later successful request
alone does not prove the disconnected stream released its resources.

## Evidence and disposition

Record findings by masked alias and source location. Validate actionable findings
with a focused behavioral probe where practical, apply the same probe to all
outputs for that task, and label it post-hoc. Never replace original acceptance
scores with expanded checks. Keep partial artifacts from failed turns identifiable
in the private mapping; a good partial implementation is not successful completion.

After the source review, unmask identities to join findings with timing, requests,
tool use, scope, failure/recovery, and context observations. Promote a candidate
only for a demonstrated benefit on new tasks without an unacceptable regression.
One task or three repetitions cannot establish universal superiority, stable
failure rates, or meaningful tail-latency percentiles.
