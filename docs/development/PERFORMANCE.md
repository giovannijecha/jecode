# Performance checks

These are bounded synthetic measurements, not a multi-day provider soak or a
promise about every workspace. They use original Rust test code, the standard
library and Windows system tools. No live OpenRouter requests or personal
configuration are used by these opt-in probes. The separate harness audit below
also records authorized live-model trials.

## Harness audit, 2026-10-04

The local audit evidence is retained under ignored
`target/harness-contract-20261004/`, including production source snapshots,
isolated fixtures, driver builders, raw request metrics and independent graders.
The driver uses the production agent, tools and HTTPS transport, with a copy-only
context-capacity override and content-free per-request measurements. Live trials
select `stealth/space-bunny-alpha`, medium effort, and isolate session storage.
They read the configured key in memory without copying or printing it.

Earlier live traces spent 1.7% to 4.7% of turn elapsed time inside tools. A native
fingerprint probe ran 30 warm samples per size, scanning 12 files twice. At 4 KiB
per file the median was 3.915 ms (2.393 to 5.793 ms); at 1 MiB it was 9.978 ms.
Those traces observed 11 to 12 files, each 46 to 4,846 bytes. These measurements
did not justify skipping content comparisons based on unchanged metadata.

The preservation trials identified request waste rather than a filesystem
bottleneck. Eight full protection entries occupied approximately 3,926 JSON bytes
in execution facts, while a failed second compaction allowed only 2,254 bytes for
continuity memory. The native prompt projection now retains actionable states,
paths and history references while leaving full baselines and reasons in durable
history. Attaching a history reference no longer re-sends unchanged baselines.
Byte counts are transport sizes, not exact token counts or speedup estimates.

Compaction allocation also calibrates projected fixed costs from the provider's
measured token density with a threefold margin and the original byte estimate as
its ceiling. Appended growth now uses that same measured density and margin;
missing calibration falls back to byte counting. A context
rejection drops calibration and follows the existing recovery path. Synthetic
checks cover fallback, calibration persistence/reset, unmeasured additions and
carried memory. Live acceptance results and limitations are in the local audit's
`RESULTS.json`; repeated runs are stochastic, and concurrent runs do not establish
isolated latency improvements.

The same driver build completed quick, formatting, forced-small-window long work
and interruption/resume trials. Independent checks passed 16/16 for the quick fix,
47/47 after formatting and 58/58 after resume. The long task completed seven
compactions without repeating its three diagnostic stages, but initially passed
only 56/58 behavioral checks. A generic review of the original behavior found and
corrected two error-output regressions; acceptance then passed 58/58. This is a
review-assisted result, not first-pass completion.

Later builds corrected retained-completion identity, proof freshness, context
calibration on resume and current system-instruction projection. Each diagnostic
build has a source manifest; the final source was not used for every earlier live
scenario. Further follow-ups exposed two remaining boundaries: validated memory
could exceed the budget after extending a conversation under a forced 24,000-token
window, and the model could omit a newly added test from a broad preservation
scope. Neither failure altered the existing registered baselines. Native protection
enforces registered paths; inferring complete paths from natural language remains
a model responsibility. The audit retains failed attempts and separately labels
the formatter trial explicitly directing registration of the current test directory.

## Harness follow-up, 2026-10-05

Evidence is retained under ignored `target/harness-next-20261005/`. The follow-up
first reproduced the broad-scope defect with a fresh real-model conversation:
after adding a CLI test, a generic formatter request completed but changed that
test, leaving only five of six required file hashes unchanged. Early scope
review blocked project operations before execution, listed related unregistered
paths and let the model register newly covered files. Two treatment runs with
the same generic request preserved all six hashes and passed 58/58 independent
behavioral cases. Refusals with `outcome:"not_started"` are excluded when counting
actual formatter executions. The inventory is bounded and does not infer or
automatically register a natural-language preservation contract.

An exact historical journal replay then exposed an incorrect explicit exclusion:
the model treated a test created in a previous request as editable, despite the
new request preserving all existing tests. It passed 58/58 functional cases but
preserved only five of six required hashes. Directory declarations now retain
the original registered scope across requests and require existing new members
to be recorded before operations. Status exclusions cannot discard that scope;
an authorized newer release remains possible. Individual releases do not waive
other members. The subsequent live replay preserved all six hashes, passed 58/58,
ran the formatter once and retained the original three diagnostic markers once
each. Its source revision and final native checks are recorded separately.

The earlier continuity failure carried 46 completed entries: about 8 KiB of the
10 KiB ledger was completed work, alongside about 2 KiB of constraints. The full
validated ledger now remains in the saved session while its active model view
can shorten and archive completed entries, exposing `history:memory` for complete
pagination. Constraints and unfinished work stay inline and retain their exact
wording. Recent proof selection uses original evidence indices, rather than the
array order produced by native carry. Synthetic checks retain 250 completed
entries through small-window compaction and resume without repeating actions.

The successful historical replay retained all 46 original completed identities
with every original evidence reference and all 12 original constraints. Its full
ledger grew to 38,140 bytes. A final-source native resume exposed a 7,307-byte
active view within a 7,705-byte budget and refused the archived original command
at `history:109` without changing its marker file. The guard compares exact Bash
text and requires a reason for deliberate repeats; it is not an atomic
exactly-once guarantee or shell-script equivalence check.

Live runs remain stochastic. One intermediate formatter run returned three
empty provider responses and ended incomplete with its session preserved;
an independent later run completed. An initial export-only historical replay
omitted token-density calibration and hit a different capacity failure. That
attempt is retained separately; the valid comparison uses the exact original
journal prefix, including all saved context fields, in isolated session storage.
The successful historical replay took 995,506 ms with 10 compactions at the
forced 24,000-token window. Complete content-free sample timings attributed 75.35%
of its sequential model-request time to summary calls. Both that cost and
occasional drift toward the earlier task remain improvement targets. They do
not justify a whole-task speedup claim. Shorter driver22 experiments passed
16/16 quick cases and 47/47 formatting cases; driver23 resumed that workspace
and registered the newly added test before its fresh check. The final driver24
also resumed and checked the workspace. The final production
gates passed 515 tests, with 10 opt-in measurements ignored.
See the follow-up's `RESULTS.json` for the executed build versions, first-pass
grades, completed trials and remaining limitations. These runs do not measure
isolated task latency or establish multi-day continuity.

The October 5 Codex source comparison isolated a separate summary-packaging
problem: the summary path subtracted UTF-8 byte lengths from a token capacity.
The initial fix converted that capacity using measured density and a threefold
margin. The measured follow-up uses a separate twofold margin for summaries and
retains the threefold margin for live projection and growth. It checks the
serialized summary messages, including escaping and
repair text, before sending. Missing calibration still uses one byte per token;
provider context rejections still reduce the portion without advancing it.

An owned loopback replay of the same native 149-message journal prefix compared
frozen production24 with this change in A/B/B/A order, twice per variant. One
compaction made 18 controlled summary requests before and 4 after; aggregate
message JSON fell from 369,488 to 131,559 bytes. All 39 projected transcript
records were reassembled with matching hashes and contiguous byte offsets,
all 149 original messages stayed unchanged, and all 46 completed identities
with their original evidence and 12 exact constraints survived. These are
packaging and native-continuity measurements using controlled responses, not
model-quality or real-task latency measurements. Initial live summary and
availability requests each received three empty HTTP 502 responses, but Bunny
later recovered with the same Jecode transport and parameters. A later frozen
control made 13 attempts and stopped on invalid continuity JSON, leaving its
context unchanged. Two updated live compactions completed in 97,775 and 139,270
ms with 5 and 6 attempts, including one and two repairs. Both kept all 149
original messages, 46 original completed identities with their evidence and
12 exact constraints. These are compaction checks, not completed whole-task
latency comparisons; the failed control cannot establish a speedup ratio.
## Reproduce

Run the opt-in checks from the repository root on Windows with Rust 1.95.0.
Release builds exclude Cargo
compilation from the timed spans. Keep the test process serial so memory samples
are not mixed with another workload:

```text
cargo test --locked --offline --release sessions::measurement_tests:: -- --ignored --nocapture --test-threads=1
```

The checks write CSV measurements under ignored `target/measurements/`. Each run
replaces the corresponding CSV; copy it within `target` before a comparison.
Filesystem and process caches are not cleared. The session workload builds a
new isolated store for each sample. Process samples alternate direct and
supervised execution after a separately measured cold start and compare stdout
and exit status.

## Session history

Executed on Windows `x86_64-pc-windows-gnullvm`, 2026-10-01, with three samples
per size. Messages alternate user and assistant, with 128-byte text payloads;
checkpoints append batches of 50 messages and include the actual sync and
summary-cache writes. Loading, resuming, context projection and display-record
reconstruction check the complete expected history.

| Operation | 1,000 messages | 5,000 messages |
| --- | ---: | ---: |
| Append all checkpoints | 137–183 ms | 550–752 ms |
| Load journal | 1.28–1.45 ms | 5.70–10.49 ms |
| Resume and save checkpoint | 6.72–11.21 ms | 22.40–25.92 ms |
| Project active context | 0.39–0.49 ms | 1.88–2.11 ms |
| Estimate context | 0.73–1.04 ms | 4.14–4.58 ms |
| Reconstruct display records | 0.12–0.18 ms | 0.77–1.24 ms |
| List one cached session | 0.32–0.50 ms | 0.29–0.42 ms |
| List one uncached session | 5.67–11.02 ms | 8.69–12.78 ms |
| Journal size | 169 kB | 845 kB |

At the final 5,000-message sampling point, working set was 27.6–27.9 MB and
private memory 24.1–24.3 MB. The test holds several copies of history and records
at once. Samples use one process whose allocator and peak working set retain
earlier work; they neither measure the full TUI's RAM nor establish a memory
leak. Original history intentionally remains available after context compaction.

This workload supports the current bounded journal growth and fast cached
listing. It does not measure thousands of separate saved sessions, large tool
transcripts, every checkpoint frequency or a cold disk. It does not justify a
new storage architecture on its own.

## Windows process startup

The probe runs the system `hostname.exe` without arguments and drains completion
and both output streams. Direct execution includes process creation and output;
supervised execution also includes ownership protection and completion status.

The original PowerShell/Add-Type supervisor averaged 479.5 ms across three warm
samples (435.2–553.6 ms). Reusing the compiled owned supervisor averaged 98.8 ms
across nine warm samples (76.1–116.5 ms), about a 79% reduction in elapsed time
for this short command. Direct execution remained 6.7–18.5 ms. Three cold
compile-and-run samples averaged 548.9 ms (392.2–629.4 ms).

The owned C# source is compiled by Windows PowerShell into the user-scoped
`~/.jecode/cache/process/` directory, with a source-version key and atomic
publication. A process reuses that executable for later commands; new launches
can reuse the same source version. Compilation has a 15-second deadline and
responds to cancellation, including while waiting for another initializer.
Compilation or executable-launch failure retains the original PowerShell path.
The cache is generated code, contains no credentials and can be recreated.

These small samples expose a fixed startup cost; they do not establish a tail
latency percentile or a 79% improvement for long-running commands or requests.

## Current-session request phases

The October 5 current-session follow-up separates packing,
curl phases, first model progress, tool spans and other native work. Two minimal
fresh baseline responses completed in 1,895.552 / 2,378.564 ms. Final-source
interrupted work followed by a changed-goal audit passed 70/70 scoped checks in
the latest two repetitions, in 72,247.531 / 58,724.928 ms, with two/one summary
attempts and six ordinary attempts each. Request spans occupied 98.34% / 98.43% of those
audit times; they include provider waiting and local stream/persistence work.
An earlier current-source run delivered correct artifacts with an incomplete
report; report language also varied. Those limits remain documented.

Minimal responses before the final proof-boundary fix took 6,268.149 / 5,548.963 ms with zero tools and
no summary. The first recovered an HTTP 502 through a requested 2,000 ms
backoff; its residual time includes that wait, not just loop/checkpoint work.
The findings do not establish server-only timing, a general speedup or a tail
percentile. No transport or tool concurrency optimization was added on this
evidence.
