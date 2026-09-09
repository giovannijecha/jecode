# Local agent comparison

This development-only pilot compares a pinned Codex CLI with an immutable Jecode
source snapshot through their real terminal interfaces. It measures complete task
outcomes, not pure inference speed or a universal ranking. It is separate from
the six synthetic performance probes and never ships in the npm runtime.

## Start here

Run `python3 -B dev/benchmarks/head-to-head/offline.py` from the repository on
Linux or WSL before editing the harness or evaluators. It runs Python tests,
benchmark correctness calibration, and the self-contained `config-edit` and
`file-server` evaluator calibrations without credentials or generation. It needs
Python 3.11+, native Node.js 24.18+, Git, PTYs, and a writable `/var/tmp`; cleanup
tests intentionally enforce that temporary-directory boundary. Ordinary CI runs
this entry point in a separate Linux job.

| Area | Status and entry point | Inputs and retained decision |
| --- | --- | --- |
| Shared infrastructure | Maintained: `offline.py`, `prepare.py`, `run.py`, `batch.py`, `analyze.py`, `verify.py` | Offline checks are self-contained; live runs need a prepared private laboratory and authorized accounts. |
| Public held-out tasks | Maintained: `tasks/config-edit/validate-evaluator.mjs`, `tasks/file-server/validate-evaluator.mjs` | Bundled fixtures and reference solutions; use explicit task selection for new comparisons. |
| Ledger pilot | Completed: default `run.py` task and `acceptance.mjs` | Bundled fixture; preserved to reproduce the initial pilot, not a broad quality claim. |
| Grouped edits | Completed: `planning_report.py` | Historical private planner starters and frozen runs; the instruction was [integrated](../../validation/HEAD-TO-HEAD-INTEGRATION-2026-09-08.md). |
| Contract, follow-up, and durability | Historical: quality/follow-up/durable reports and `recover.py` | Prepared private planner/durable starters and saved runs are required; see the [durability evidence](../../validation/HEAD-TO-HEAD-DURABLE-2026-09-08.md). |
| Work state | Completed, not promoted: `daily_report.py` and `DAILY-EXPERIMENT.json` | Private experimental snapshots and saved runs; [the decision](../../validation/DAILY-DRIVER-2026-09-08.md) remains rejection of promotion. |
| Transport experiments | Optional: `transport_batch.py ROOT` | An explicitly prepared laboratory and authorized live execution. |

Historical commands do not reconstruct missing private inputs. In particular,
`precheck.py ROOT` retains historical defaults: select
`--tasks config-edit file-server --variants baseline` explicitly for the public
baseline tasks. Offline calibration complements, but does not replace, the live
preflights and frozen-source verification below.

## Protocol

- Use native Linux executables and native Linux files in the same WSL2 guest.
  Record OS, Node, both client versions, source hash and relevant settings.
- Use the same account, `gpt-6-astra`, high effort, fresh
  sessions, the identical prompt, and independent copies of the same fixture.
  Account equality is recorded as a boolean; identifiers and secrets stay local.
- Disable Codex Fast mode and select its default service tier. Jecode leaves the
  optional tier field unset. Record this difference: without response evidence,
  the actual backend routing/priority is unverified, even with the same account.
- Disable delegated agents, memories, plugins, apps and web access for this
  offline task. Preserve each client's native system prompt and tool definitions.
  Differences in that model-visible context are part of the product comparison.
- Codex uses its workspace-write sandbox with approval policy `never`. Jecode's
  real `/permissions` menu grants edit/write/command access for the disposable
  session. Its approved shell remains unsandboxed, as in production. These are
  different security mechanisms and must not be reported as identical isolation.
- Run one agent at a time. Start with one pilot pair, then use a counterbalanced
  order for repeated measurements. Keep failures, timeouts and failed checks in
  the results; never discard them just because a retry performs better.
- Finish the fixture and acceptance checks before sending the first task. The
  evaluator stays outside both workspaces. Fix a defective evaluator openly and
  rerun it on both saved outputs; never tune it to prefer a client's solution.
- Compare completion time only alongside the common acceptance results and a
  review of scope, regressions, error handling and test quality. An agent's own
  test count is supporting evidence, not a comparable quality score.
- For new campaigns, freeze held-out long tasks, acceptance checks, and a blind
  review rubric before generation. Measure scope compliance and time to a correct
  outcome; report failed and unfinished tasks separately from completed timings.

The fixture starts with 9 of 56 acceptance checks passing. `PROMPT.md` defines the
public task; `acceptance.mjs` independently checks CSV parsing, integer money,
date/account filtering, aggregation and CLI behavior. A single small task cannot
establish general coding quality or long-session reliability. Add held-out bug,
feature and refactoring tasks before using this to judge broad improvements.
The initial smoke test is stored as a `.template` file and materialized without
that suffix when copying the fixture. Its contents stay identical; this keeps
Node's repository-wide test discovery from counting a pilot fixture as a Jecode test.

## Environment and execution

Requirements: Linux, Python 3.11+ standard library, Git, a supported native Node.js,
and a pinned native Codex npm installation. Choose a private task directory under
`/var/tmp`; unlike a tmpfs `/tmp`, this preserves evidence across guest shutdown.
Keep application roots, homes, workspaces and reports on the Linux filesystem.

1. Create the private directory. Install the exact Codex version into its `codex/`
   prefix with npm `--ignore-scripts`, recording package-lock integrity.
2. Run `prepare.py --root ROOT --source CHECKOUT --node NODE` with explicitly
   selected `--codex-auth AUTH_JSON` and `--jecode-account ACCOUNTS_JSON` files
   for authorized local account reuse. It creates private copies; it never prints
   their contents or modifies the originals. Interactive sign-in is needed when
   no suitable account is available. Do not copy API keys into this experiment.
3. Validate the source snapshot's typecheck and tests. Copy this harness into the
   laboratory before running it, so fixture/evaluator reads also use Linux files.
4. Run `python3 run.py --root ROOT --client jecode --name preflight-jecode --preflight`
   and the corresponding Codex command. Both must reach the intended model and
   composer; preflights send no user task. Codex may prewarm its connection.
5. Run each client with a new unique name and without `--preflight`. The default
   task deadline is 1,200 seconds. The driver submits the same bracketed paste,
   observes completion, exits the TUI, and invokes the external evaluator.

For a new six-run sequence, `python3 batch.py --root ROOT --prefix trial --clients
jecode codex codex jecode jecode codex` records the order before starting. It runs
serially in one parent process so WSL cannot idle-shutdown between runs. Failed
tasks remain in the results; setup/harness failures stop the batch for inspection.
It never resumes or overwrites a previous batch implicitly. Add `--precheck`
to require the plan's offline fixture/source/harness and native sandbox checks
to pass before any timed run starts. Preparation runs with the selected Node
directory in PATH. A failed precheck returns without declaring or launching
the timed batch; fix the environment before trying a new measurement.

Every run owns a fresh workspace and data home. Names are never reused or
silently reset. Preserve the manifest, outcome, acceptance results and relevant
transcript evidence. Credentials and raw captures must remain untracked, private
and local. After the comparison, remove temporary credential copies and retain
only the evidence needed to reproduce/review conclusions.

The PTY parser advances through each chunk with a read index and retains only an
incomplete escape suffix, capped at 4,096 characters. Malformed or oversized
incomplete sequences fail the harness; cleanup still terminates and reaps its
child. Run and recovery outcomes retain numeric `terminalLifetime` observations:
chunk/byte counts, largest chunk, chunks above 16 KiB, and total/maximum parser
time. These include setup and cleanup, exclude raw-log writes, and must not be
subtracted from model inference time. Older captures lack these observations;
the parser optimization alone does not quantify bias in earlier comparisons.

`python3 cleanup.py ROOT` removes only account copies in the private `/var/tmp`
laboratory after all runs have settled. It checks resolved paths before deletion
and leaves original account stores, projects and measurement records untouched.

After the live runs, use `python3 verify.py ROOT --own-tests` to recheck the
frozen inputs, source snapshot, acceptance outcomes, project tests, dependencies,
links and Git history. It preserves original outcomes and writes separate
verification artifacts. Run `node validate-sessions.mjs ROOT` to decode Jecode
checkpoints with the measured source's production codec. Neither command sends
provider requests. `python3 analyze.py ROOT` summarizes the local evidence;
`--live` also shows unfinished runs without treating partial JSONL tails as records.

## Measurement boundaries

The clock begins at prompt submission after startup and permission setup. End is
the observer detecting a settled Jecode checkpoint or Codex's documented
`agent-turn-complete` notification. Both are polled through the same PTY loop;
small observation and notification overhead remains. Acceptance execution is
timed separately. Startup, sign-in, package installation and manual preparation
are excluded; this is a ready-composer comparison, not cold launch latency.

The driver drains terminal output continuously. Its minimal screen model only
recognizes setup states; it does not measure physical terminal painting or
accessibility. Both interfaces use a 140-by-40 PTY and `NO_COLOR`.

Jecode's existing context recorder captures request preparation, adapter time,
first output, usage and transport state. Codex's documented OTLP JSON exporter
sends to a local loopback collector, which retains whitelisted timing/usage
fields and discards prompt, email, account ID and tool-output values. Export is
asynchronous: event timestamps, not batch arrival time, determine event order.
Neither client is pointed at a replacement model endpoint or proxy.

New trial manifests hash the evaluator's `.mjs` module tree as well as its main
entrypoint. Verification rejects changed or added helper modules. Older trial
manifests retain their recorded entrypoint-only scope; never imply they covered
unrecorded dependencies.

Metric boundaries differ. In particular, Codex WebSocket send duration is not
equivalent to Jecode's full `providerMs`. Codex's event timestamps can bracket
serialized WebSocket sends and response completions; the analyzer reports these
intervals only when every pair is unambiguous. They include provider and network
time, and still are not pure inference measurements. Its turn-level TTFT is not
the same boundary as Jecode's per-request first semantic stream event.

Pair requests before filtering by submission time: a prewarm request can start
before submission and complete afterward. Exclude only an unambiguously paired
pre-submission completion from task usage and report that count; keep unknown
boundaries visible. This does not subtract overlapping warmup from elapsed time.
Reconcile task usage with the saved rollout before comparing token totals.
The analyzer also counts events carrying an `error.message` attribute name,
without retaining its value. A `response.completed` event without usage can
report an error; collector parse failures and provider errors are distinct.
Keep recovery time in task elapsed and error-bearing intervals in attempt timing.

Codex tool telemetry includes the model-facing `exec` wrapper and nested host
operations. The analyzer counts model calls separately from those operations;
adding their durations would count overlapping work twice. Do not divide a whole task's token total
by elapsed time and call it decoder throughput. Distinguish missing counters
from zero, cache reuse from billing savings, and local elapsed time from provider
inference. Report unavailable comparable measurements explicitly.

Harness checks: `python3 -m unittest discover -s . -p 'test_*.py'` from this
directory. These use loopback fixtures and make no provider calls. Jecode runtime
verification remains the repository's normal typecheck/test/check workflow.

`node explore-ledger.mjs WORKSPACE` is a supplementary post-hoc correctness probe,
separate from the frozen 56-check scorer. It uses a fixed seed for 400 generated
ledgers (quoted punctuation, embedded line endings, Unicode and unusual account
names) plus 22 calendar/money boundary checks. Expected values come from the
generated rows, without parsing the CSV under test. Save its seed, script hash
and results under a separate filename; never replace the original acceptance
outcome or treat these added checks as having been specified before the pilot.

The [2026-09-07 report](../../validation/HEAD-TO-HEAD-2026-09-07.md) retains the
first six live attempts, including a Jecode timeout and the measurement limits.
The [progress-deadline follow-up](../../validation/HEAD-TO-HEAD-PROGRESS-2026-09-07.md)
records the fix, four new live attempts and independent correctness probes.

The [planning experiment](PLANNING.md) adds cache debugging and a planner spread
across ten implementation modules. `run.py --task cache|planner --variant
baseline|grouped` selects a fixed task and independently hashed Jecode source.
The [integration repeat](../../validation/HEAD-TO-HEAD-INTEGRATION-2026-09-08.md)
subsequently validated and integrated the grouped instruction. Historical frozen
variants remain reproducible; `freeze_grouped` requires the original prompt and
refuses a source that already contains the instruction. Do not insert it twice
or label the current production prompt as the historical baseline.
The [quality follow-up](../../validation/HEAD-TO-HEAD-QUALITY-2026-09-08.md)
retains a failed baseline despite its passing partial artifact. `verify.py`
checks original settlement as well as artifact/provenance checks; a successful
test rerun must not turn an unfinished task into a successful trial.
The verifier also requires every run in a declared batch to have a settled
artifact report; an unstarted or missing trial cannot silently shrink the sample.

Before another subprocess-heavy comparison, run `spawn-probe.mjs` with the
selected Node outside and inside the selected native sandbox. The recorded
Codex `:workspace` profile returns `EPERM` for pipe capture even though the child
status can be zero. The probe is offline and does not change permissions or
contact a provider. Resolve or declare this execution difference before timing;
do not change sandbox restrictions during an already-declared batch.

`environment-probe.py --output NEW_DIRECTORY --codex PINNED_CLI --node NODE`
compares default, Unix-socket and network-enabled workspace profiles without
provider calls or changing personal settings. The
[integration protocol](INTEGRATION.md) declares a separate network-enabled repeat.
`prepare.py --codex-command-network` freezes this opt-in choice in environment
metadata; the runner selects that named profile while retaining workspace writes.
`precheck.py ROOT` defaults to the historical cache/planner tasks and grouped
variant. `--tasks` and `--variants` select another declared experiment. It checks
the expected incomplete starting fixtures, every harness test, source types and
tests, subprocess capture, and the starter tests inside native Codex's selected
permission profile. Keep unsuccessful preparation artifacts;
they are harness failures, not model-task observations.

`recover.py --root ROOT --name recovery-cache --task cache --variant baseline`
runs a separate recovery trial: interrupt after a persisted write, exit, reopen
with `-c`, verify no file/history change or generation before new input, then
continue and evaluate the complete task. Its elapsed time includes interruption
and restart; never pool it with uninterrupted trials. The recovery report
separates these phases. `planning_report.py ROOT` describes edit grouping from
canonical messages; it attributes request durations only when counts and outcomes
establish an unambiguous single-turn alignment.
The [planning results](../../validation/HEAD-TO-HEAD-PLANNING-2026-09-07.md)
retain six uninterrupted runs and a separate recovery run. `explore-cache.mjs`
is an output-derived reentrancy probe, outside the frozen acceptance score.

The [quality review](QUALITY.md) adds identity-masked artifact copies, cross-suite
checks and common cache/graph probes. `quality.py` never edits saved outputs;
review findings remain separate from original acceptance scores. The follow-up
planner task uses one frozen working starter for every participant and a new
evaluator that also requires the original contract. Its fixture is materialized
only in the private Linux harness by `followup.py`, not stored as generated code
in this checkout. [Quality results](../../validation/HEAD-TO-HEAD-QUALITY-2026-09-08.md)
record provenance, defects, counterevidence and promotion limits.

`supplementary.py ROOT` runs the shared planner probes and
`representation-probe.mjs` against settled outputs, preserving their own hashes,
exit codes and separate results. The representation cases were derived from
source review after the integration trials began. They must not replace the
frozen acceptance score or be described as blind or specified before that batch.

The [durable execution experiment](DURABLE.md) extends the same planner with
asynchronous execution, cancellation, atomic checkpoints and restart. It compares
the current grouped baseline, a separately frozen `contract` instruction and
native Codex using `DURABLE-EXPERIMENT.json`; `batch.py --timeout 1800` records
the longer deadline in the batch manifest. `durable.py ROOT PREVIOUS_ROOT`
materializes the exact private starter and rejects a different manifest. These
historical fixtures require the retained laboratory; they are not CI inputs.
`scenarios.py ROOT --variant contract` freezes the instruction experiment.
Its [report](../../validation/HEAD-TO-HEAD-DURABLE-2026-09-08.md) records evaluator
review separately from original scores. `durable-failure-probe.mjs WORKSPACE`
checks non-Error callback failures and capture validation after source review; it is supplementary,
not part of the predeclared acceptance score.
`durable-queue-probe.mjs WORKSPACE` counts Promise initialization events without
a corresponding resolution hook, with one slow worker and many fast independent
tasks. The [Node hook](https://nodejs.org/docs/latest-v22.x/api/async_hooks.html#promiseresolveasyncid)
does not establish fulfillment. This is not a heap measurement or an
uninstrumented execution-speed score.

`resume-failure.py ROOT --failed ORIGINAL_RUN --name NEW_RUN --source SNAPSHOT`
continues a settled failed trial in a copied workspace, outside timed rankings.
It stages a copy of the original session and uses the runtime store to load and
re-publish the same conversation for the copied workspace. It is a development
experiment, not a user-facing session import command. The original trial remains
unchanged. Verify no generation or file change before new input, then record the
continuation and re-run the external checks separately. A recovered result never
replaces the original failed outcome or repairs its timing score.
`verify.py` lists these copies separately; they cannot satisfy a declared trial.

References: [Codex CLI](https://learn.chatgpt.com/docs/codex/cli),
[Codex telemetry and notifications](https://learn.chatgpt.com/docs/config-file/config-advanced),
[WSL filesystem guidance](https://learn.microsoft.com/en-us/windows/dev-environment/wsl-interop#performance-considerations).

## Daily-driver experiment

`DAILY-EXPERIMENT.json` declares eighteen serial trials: two new tasks, three
configurations, three repetitions. `config-edit` covers JSON-pointer operations,
validation, staged migration, permissions, cancellation and rollback. `file-server`
covers HTTP validators, byte ranges, path boundaries, streaming and shutdown.
The same frozen fixtures and literal prompts apply to all configurations.

The [completed investigation](../../validation/DAILY-DRIVER-2026-09-08.md)
records all eighteen trials, masked output review and supplementary failures.
The work-state variant remains experimental and is not promoted.

These Linux task evaluators use real symlinks, HTTP sockets and POSIX signals.
Calibrate each with `node tasks/TASK/validate-evaluator.mjs`: the reference must
pass every check and each deliberate mutation must fail. Reference code stays
outside participant workspaces. The calibrated checks are still a finite sample;
resource bounds, code clarity and untested races require separate output review.

`daily_variants.py ROOT --variant work-state` freezes the optional
[work-state candidate](../../experiments/README.md), registers it through the
normal permission control plane, and binds it to canonical history. Registry
expectations in three candidate-only tests include the extra tool. It changes
no production file or historical snapshot. `--variant http` makes a separate
one-line forced-HTTP transport variant for protocol probes, not a shipping flag.

Run `precheck.py ROOT --tasks config-edit file-server --variants work-state`
before preflights and `batch.py --root ROOT --prefix daily --plan
DAILY-EXPERIMENT.json --timeout 1800`. Stop on preparation errors. A corrected
preparation receives a fresh laboratory; retain the rejected one as setup
evidence, not a timed participant trial.

`transport_batch.py ROOT` runs six serial synthetic account transport probes in
counterbalanced order, with two requests per turn. It preserves the HTTP turn
routing state and verifies the actual transport. Run these separately from the
TUI comparisons and heavy local checks. Results measure complete provider-adapter
intervals, not pure server inference or a statistically established failure rate.

After all eighteen trials settle, `verify.py ROOT --own-tests` rechecks outputs
and provenance separately from original acceptance. `daily_report.py ROOT` keeps
every declared attempt and compares elapsed means only when both configurations
complete all repetitions with full original acceptance. Failed-attempt duration
and successful-subset duration remain separately labeled. Neither implies an
overall quality ranking.

`daily_review.py ROOT NEW_REVIEW_DIRECTORY` makes randomly masked copies of all
outputs, including partial artifacts. Keep its private `identity.json` closed
until source findings have been recorded using [the review rubric](DAILY-REVIEW.md).
Join those findings to client identities only afterward. Additional probes are
post-hoc evidence and must not replace the original acceptance scores.
Optional `--round 1`, `2`, or `3` selects exactly that contiguous six-run group
from the original plan, including failed outputs. Each needs a new review directory.
This permits light source inspection while later trials run. Keep every identity
map closed until all eighteen source reviews are recorded, and run behavioral
probes only after timed trials finish. No review feedback reaches a participant.

`daily-probes.mjs TASK MASKED_WORKSPACE` records supplementary rollback, file-open
counts, conditional-list tolerance, invalid dates and premature EOF after source review. Its write
fault affects `fs.promises.writeFile` and `FileHandle.writeFile` while renames
remain available; it does not simulate every filesystem failure. Open-call
counts describe the monitored API, not all kernel I/O or elapsed performance.
The EOF probe overrides `FileHandle.read` after metadata succeeds, then checks
the bounded error response and descriptor closure. An unexercised hook is
inconclusive, not a pass or failure.
Malformed-tag recovery is recorded descriptively, without assigning pass/fail;
empty-list-element handling and dates have separate expected outcomes. The
[investigation report](../../validation/DAILY-DRIVER-2026-09-08.md) records why
these post-hoc checks were added and their protocol references.
These probes are separate from original acceptance and must run uniformly on
every output of the relevant task, after the timed batch has settled.
`daily_probes.py ROOT REVIEW_DIRECTORY...` enforces that all eighteen declared
outputs are represented once, supplies isolated temporary homes, bounds each
probe process, and checks that participant workspaces remain unchanged. Its exit
status reports runner integrity; inspect individual results for observed
robustness differences. Identity maps are not needed to execute these probes.
