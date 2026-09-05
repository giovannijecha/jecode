# Validate a release candidate

Use this protocol to decide whether a candidate is ready for promotion. The
[compatibility contract](../../docs/COMPATIBILITY.md#release-candidate-gate)
owns eligibility, minimum soak duration, and restart rules. The
[release runbook](../../docs/RELEASING.md) owns publishing. This page defines
repeatable checks and the evidence to collect; it is not a completed report.

## Prepare the record

Start with an exact version, commit, package integrity, and CI run. Record
local exploratory results separately from the candidate's acceptance results.
Use the installed candidate for interactive checks. Use the matching source
commit for the [automated gates](../../scripts/README.md) and
[performance probes](../benchmarks/README.md).

Create disposable workspaces and an isolated development `JECODE_HOME` before
testing crashes, malformed data, or permission boundaries. Use synthetic secrets
and inert files for negative tests. Live provider checks use explicitly
authorized accounts; record the provider and model, never credentials or
account identifiers. Do not tamper with production sessions or revoke a real
credential to simulate an error.

Record each check as `pass`, `fail`, or `not run`. A pass names the observed
result and its evidence. A skip or unavailable environment remains `not run`.
Use one reviewed record linked from the candidate and promotion pull requests;
do not duplicate it in architecture, changelog, and direction documents.
Raw captures follow the [temporary-output lifecycle](../README.md#temporary-outputs).
Retain the sanitized evidence needed to review the result before removing raw
files. Report security findings through the private route in
[SECURITY.md](../../SECURITY.md).

## Automated baseline

- All six supported OS/Node CI combinations pass for the candidate commit.
- Type, coverage, zero-runtime-dependency, source-tree, package, and installed
  executable checks pass. Record platform skips and confirm that the relevant
  behavior runs on another supported platform.
- Every confirmed defect fixed during stabilization has a focused regression
  check. Do not lower coverage or timing limits to accept an unexplained failure.

## Live provider paths

For each stable provider in the [supported surface](../../docs/COMPATIBILITY.md#providers),
record at least these outcomes in the installed candidate:

| Check | Required result |
| --- | --- |
| Connect and select | Access status and model route agree; secrets stay out of transcript and export. |
| Stream and use tools | Text streams, a read completes, and a harmless write in the disposable workspace requires the applicable approval and shows its result. |
| Interrupt and continue | Interrupt a streaming response, then send another turn; no stale output or issued tool is replayed. |
| Mid-turn controls | Enter guidance and open a menu during generation; guidance is consumed at a safe boundary and model changes apply to the next turn. |
| Access or transport failure | An isolated invalid credential or interrupted connection produces bounded feedback, preserves settled history, and permits recovery. |

Exercise the experimental OpenAI Account route separately: browser sign-in,
cancellation, resumed access after restarting Jecode, a streamed tool turn, and
interruption. Record transient upstream limitations explicitly. Do not use a
live account to deliberately provoke rate limits; automated HTTP fixtures cover
retry, rate-limit, and malformed-stream cases.

## Terminal and accessibility paths

Complete the keyboard and rendering checks on a real terminal host on Windows,
Ubuntu, and macOS. Record terminal name/version, OS, Node, dimensions, and input
method. WSL checks are additional evidence, not a replacement for Windows input
or macOS terminal checks. Cover both supported Node lines across these sessions;
CI covers their full OS cross-product.

| Check | Required result |
| --- | --- |
| Normal and narrow frames | Inspect 120x40 and 60x20 cells, resize while streaming, and restore size. No stale rows, lost focus, or content outside the frame. |
| Composer and menus | Exercise multiline paste, Unicode graphemes, word deletion, model filtering, permission changes, approval focus, and Esc restoration using the documented keys. No draft loss or accidental confirmation. |
| Transcript | Scroll during output, return to the live tail, and toggle expanded evidence. User text, reasoning, tool results, and composer remain distinguishable. |
| Colour and motion | Repeat with `NO_COLOR`, reduced motion, and both together. State, focus, approval choices, and tool errors remain readable without colour or animation. |
| Terminal restoration | Exit normally and interrupt or crash the disposable run. Ordinary exit restores input and screen. After an abrupt kill, verify recovery in a fresh terminal and record any reset needed in the original host. |

Evaluate screen-reader operation with NVDA on Windows and VoiceOver on macOS:
enter a prompt, identify streamed results, navigate choices, and distinguish
allow from deny. Record the reader version and observed failures. These are
required evaluations, not a claim of existing screen-reader compatibility;
triage inaccessible core actions under the same severity rules as other defects.

## Sessions, recovery, and boundaries

Run these checks in disposable data, then compare transcript, export, and the
saved branch. Keep automatic regression tests as the source for fixture formats.

| Check | Required result |
| --- | --- |
| Resume and branch | Restart, resume the latest session, select a historical turn, and send a new turn. The original branch survives and historical tools never execute again. |
| Context recovery | Exercise automatic and manual compaction in a long conversation. Canonical history and export remain complete; subsequent turns stay usable. |
| Partial tool loop | Kill a disposable Jecode process while a tool is active and resume. Recovery chooses the documented safe ancestor or settled state without replaying work. |
| Exclusive ownership | Attempt a second writer for the same session, then recover a stale lease after terminating its owner. No concurrent overwrite or lost history. |
| Invalid persisted data | Use existing test fixtures for truncated/unsupported checkpoints and older supported schemas. Loading migrates safely or fails without deleting the original. |
| Permissions | Deny a harmless write/command, allow an exact scope, revoke it before pending work, and start `/new`. Execution follows current permissions and reset behavior. |
| Filesystem and secrets | Run link/junction, realpath, preview-race, environment-filter, and redaction regressions. Check synthetic secrets in output and export. An approved shell retains the explicitly documented OS access. |

Automated injection is appropriate for races, invalid data, and transport errors
that cannot be triggered safely and reliably by hand. Link the passing test and
state that it was automated; it does not replace the interactive recovery checks.

For context investigations, the [development recorder](../context/README.md)
captures request estimates, provider counts, and accepted/failed/cancelled summary
attempts without conversation content. Record its source commit and any dropped
events. Keep development-launch observations separate from acceptance checks of
the installed candidate; the recorder does not establish that artifact's behavior.

## Performance

Run all six probes before and after relevant changes on the same idle machine,
Node version, power profile, and storage. Record commit, fixture sizes, timings,
and existing threshold outcomes using the [benchmark guide](../benchmarks/README.md).
Investigate a repeatable regression before accepting it; a noisy sample alone
does not establish a defect or justify raising a threshold.

During long sessions, also record input-to-visible-update latency, resident
memory at comparable settled points, and responsiveness while output is slow.
Include streaming text, large tool evidence, open menus, and scrollback. Separate
local processing, terminal painting, and provider latency. A renderer benchmark
does not measure the time until a user sees a keypress.

The [integrated TUI probe](../benchmarks/README.md#integrated-tui) supplies input-to-frame,
write, and consumer-acknowledgement measurements with memory snapshots. Its
synthetic consumer does not establish physical terminal latency or accessibility.

For latency, collect at least 100 samples per scenario after warm-up and report
the median, p95, and maximum with the measurement method. For memory, report the
transcript size alongside each sample; growth with retained history is not by
itself a leak. Explain retained growth after equivalent work/reset cycles.

Only add a CI timing gate after establishing a repeatable baseline and an
explicit regression budget. Prefer deterministic tests of bounded work and
scaling where hardware timing would be unstable. Missing end-to-end measurements
remain `not run`; synthetic results must not be presented as interactive latency.

## Soak log and decision

For each work session, record date, duration, completed turns, workspace alias,
provider/model, platform/terminal, scenario IDs, and findings. Use aliases rather
than private paths. Check totals against the compatibility gate before promotion.

Use this compact structure for the reviewed record:

```text
Candidate version / commit / package integrity:
CI run and platform skips:
Soak start / end / work-session totals:
Environment: OS / terminal / Node / accessibility settings
Check ID | environment | pass / fail / not run | observation / evidence
Findings: severity / reproduction / fix / regression evidence
Remaining gaps:
Decision: hold / eligible for promotion
Reviewer / date:
```

Use critical/high for credential exposure, boundary bypass, destructive data loss,
or similarly severe impact; medium includes reproducible broken core flows,
unrecoverable input/focus, and material performance or accessibility failures.
Low findings are bounded cosmetic or minor usability defects with a practical
workaround. Assess impact rather than choosing a severity to pass the gate.
Confirmed candidate regressions block promotion regardless of severity.

Eligibility requires every required check to pass and all blockers to be closed.
Record outstanding low-severity, pre-existing limitations with their impact.
An unavailable account, platform, or reader leaves that check pending; it cannot
be silently waived or inferred from CI. An eligible record authorizes no publish
operation by itself; follow the normal reviewed release workflow.
