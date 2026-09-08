# Context and turn continuity hardening

This development investigation follows the [daily-driver comparison](DAILY-DRIVER-2026-09-08.md).
It targets changing requirements during preparation and failure boundaries in
long conversations. It does not repeat the eighteen live trials, replace their
results, or claim a new speed or output-quality advantage over Codex.

## Scope and reproducibility

- Branch: `perf/provider-roundtrips`, base commit
  `768b686934184627cac3d7e30773f124405ae56f`, with the existing pending work retained.
- Windows: Node 24.18.0; WSL Ubuntu: Node 22.23.2.
- Frozen source: `/var/tmp/jecode-continuity-20260908-mqc16_s1/source`.
  Its 550-file `manifest.json` has SHA-256
  `e1e6cf9bd9f4e4c66b39c85309bed4da33a1c5944cbdb032ffe575cc1583625b`.
  It captures code, tests, and documentation before this report and its index
  entry were added. All captured files matched the checkout after verification.
- Tests use inert providers and loopback sockets. No live account, credentials,
  production session, external model generation, or benchmark prompt was used.

## Confirmed defects and changes

| Boundary | Reproduced behavior before the fix | Corrected behavior |
| --- | --- | --- |
| Guidance during preparation | Guidance queued during metadata, compaction, or input measurement missed the impending request. | Drain before generation and remeasure the revised input, including guidance arriving during the new measurement. No repeated summary is required. |
| Summary acceptance | Nonempty truncated/refused responses and unexpected tool blocks could become replacement memory. | Owned adapters expose a live completion outcome; compaction rejects unusable responses before creating an anchor, while accounting for returned usage. |
| OpenAI terminal validation | An event named `response.completed`, `response.done`, or `response.incomplete` could conceal a failed/error-bearing response envelope. | Fail the request without committing the response or executing its tools. Started generations are not replayed. |
| Final-output precedence | A larger streamed item list overrode a shorter nonempty final output, allowing an unissued tool to survive a final refusal. | The nonempty final output wins. Complete streamed items remain the compatibility fallback for an empty final output. |

The defects were reproduced with focused failures before changing the relevant
implementation. The summary fixture was corrected to ensure it actually reached
the summarizer; each negative case now asserts one summary request as well as
rejection. Positive controls accept complete summaries from all three wire
formats and legitimate text containing a literal truncation notice. Rejection
does not depend on searching the generated prose for keywords.

The completion outcome is optional for legacy/fixture messages, supplied by all
owned wire adapters, and excluded from session serialization and provider input.
No schema migration, context-window reduction, tokenizer replacement, model
effort reduction, mandatory planning loop, or new runtime dependency is involved.

Late guidance cannot authorize an otherwise unchanged overflow retry. Cancellation
before guidance is accepted leaves it queued; oversized accepted guidance remains
in canonical history while the over-budget generation is prevented. Existing
tool batches and requests already in progress keep their original safe boundary.

## Regression evidence

- [Preparation steering](../../test/controller-preparation-steering.test.ts):
  metadata and compaction delays, guidance during remeasurement, budget overflow,
  cancellation, and refusal to replay an unchanged context-rejected request.
- [Summary completion](../../test/summary-completion.test.ts): OpenAI incomplete
  and refusal outcomes; Anthropic token limit, pause, and refusal; Ollama length
  and content filter; unexpected tools; valid-summary positive controls; usage
  accounting and sanitized `incomplete` diagnostics.
- [Terminal validation](../../test/openai-terminal-validation.test.ts): failed
  envelopes, invalid terminal states, and final refusal replacing streamed tools.
- [Full TUI and durable recovery](../../test/app-websocket-recovery.test.ts):
  delay a summary, queue guidance through the composer, execute one file effect,
  then fail a later generation after partial output and an uncommitted tool item.
  Exit, resume, and continue. The first post-summary generation receives guidance;
  saved history and the anchor survive; the effect runs once; incomplete tools
  and text are absent from resumed model input. Resume alone sends no request.
  Both recovery variants run at 40 columns with reduced motion; the Linux run
  also sets `NO_COLOR=1`.
- [Automatic recovery](../../test/app-context-recovery.test.ts) and
  [manual compaction](../../test/manual-compaction.test.ts): rejected memory
  preserves prior context/anchors and does not trigger an immediate retry on an
  unchanged tool follow-up. [Codec coverage](../../test/session-codec.test.ts)
  verifies the live outcome does not enter durable messages.

## Verification results

| Check | Result |
| --- | --- |
| Windows `npm run check` | Pass: source tree, types, coverage, package, installed CLI |
| Windows full suite | 1,088 tests: 1,077 passed, 11 platform skips, 0 failed |
| Windows coverage | 96.22% lines, 88.92% branches, 94.28% functions |
| Package | 192 files, 1,929,993 bytes unpacked, zero runtime dependencies; installed CLI reports 0.8.7 |
| WSL focused tests, `NO_COLOR=1` | 52 passed, 0 failed or skipped |
| Isolated WSL `npm ci --ignore-scripts`, typecheck, `npm test` | Pass: 1,088 tests passed, 0 failed or skipped |
| Whitespace check | `git diff --check` passed |

The suite contains 28 additional cases relative to the preceding 1,060-test
snapshot. Linux installation, typecheck, and test logs are retained alongside
the manifest. The Windows gate log is copied there as
`jecode-continuity-check.log`; its checkout copy remains ignored.

An initial WSL `npm test` against the Windows checkout failed before tests:
Windows-installed TypeScript lacked `@typescript/typescript-linux-x64`. The
isolated copy with its own development installation resolved the environment
mismatch. That failure is retained in `jecode-continuity-wsl-test.log`; it was
not classified as a product failure or bypassed by weakening checks. The Windows
runtime build is regenerated after this attempt, which had cleaned `dist/`.

## Limits and next evidence

This demonstrates deterministic request, compaction, and recovery behavior.
It does not establish semantic completeness of generated summaries, perceived
terminal latency, screen-reader accessibility, or success across all task types.
The earlier live output-stage WebSocket 1006 remains unexplained; these fixes
must not be described as resolving its cause or implementing safe automatic replay.

The next live comparison should use new multi-session repository tasks with
frozen acceptance, changing requirements, actual context pressure, and a resume
boundary. Measure time to a verified result and review requirements beyond the
happy path. Keep the previous unsuccessful work-state experiment unpromoted and
retain failures in the comparison. No commit, PR, merge, or release occurred in
this hardening round.
