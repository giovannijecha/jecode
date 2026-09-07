# Benchmark collections and comparisons

The [Benchmarks workflow](../../.github/workflows/benchmarks.yml) runs the six
existing [probes](README.md) without providers, runtime dependencies, or a second
implementation of the measured behavior. It runs on relevant pull requests and
main updates, Mondays at 05:23 UTC, and manual dispatch. It does not publish a
package, post comments, or change the required CI matrix.

## Baseline and environment

For a pull request, the measured code is GitHub's merge commit and the baseline
is its base SHA. For a main push, the baseline is the previous tip from the push
event. Scheduled and manual runs default to the measured commit's first parent;
manual dispatch can select a different baseline ref. Both refs are resolved to
immutable commits and checked out separately on the same hosted Ubuntu 24.04
runner, with Node 24.18.0 pinned. No credentials are persisted in the checkouts.

Each probe runs three times in fresh Node processes, sequentially: baseline
first, current second. Existing warm-ups and internal sampling remain unchanged.
This exposes between-process variation as well as the probe's own measurements.
Time/order effects can remain; repeat a suspected regression, including reversed
order on a quiet local machine, before drawing a conclusion.

Reports record commit, dirty status, capture time, repetitions, exact Node,
OS/kernel, architecture, CPU model/count, total RAM, and hosted image version.
The collector forces `NO_COLOR` and uses an isolated development home. Children
receive only path/system/temp variables needed to run Node, not provider keys,
GitHub tokens, user Node flags, or the user's Jecode data.

Matching metadata does not make hosted hardware, storage, power settings, or
background load identical. Historical artifacts help investigations; a fresh
paired run is stronger evidence than comparing unrelated machines.

## Reports and outcomes

The workflow retains `baseline.json`, `current.json`, `comparison.json`, and
`SUMMARY.md` as an artifact named with the measured SHA and run attempt, for 90
days. The job summary shows per-probe outcomes and up to eight largest relative
increases per probe; the JSON comparison contains every comparable latency and
memory/output-byte metric, with median, minimum, maximum, percentage change, and
whether the sample ranges overlap. A percentage is unavailable when the base
median is zero. Overlapping ranges are not a statistical significance test.

`compared` requires matching environment, clean checkouts, repetition counts,
probe source fingerprints, and reported workload parameters. `incompatible`
withholds ratios when these differ, a schema is invalid, or there are no common
measurements. Missing/null numeric samples are listed as unavailable, never
replaced with zero. Benchmark changes deliberately start a new comparison
baseline; include new fixture/helper files and workload fields in `probes.ts`.

`failed` retains nonzero exits, existing tripwire failures, cancellation,
timeouts, and diagnostic stderr. A failed probe is not reported as an improvement.
Each child is bounded to three minutes and 1 MiB of combined output. The
collector checkpoints its output after each sample, so incomplete evidence
survives a later failure. The comparison reader accepts complete version-1
collections up to 8 MiB each, and never executes content from reports.

Collection and comparison failures make the diagnostic job fail after uploading
available evidence. That job is not a required merge check: investigate its
failure rather than interpreting it as a new performance budget. No additional
timing thresholds, automatic regression verdicts, or changes to existing probe
limits are introduced. The ordinary CI matrix tests the collection machinery.

For release acceptance, retain the reviewed evidence before artifact expiry.
These synthetic measurements do not establish physical display latency,
screen-reader usability, live-provider performance, or release-candidate soak.
The [validation protocol](../validation/README.md#performance) owns those checks.

## Local use

Create a task directory under the system temporary directory, following the
[temporary-output lifecycle](../README.md#temporary-outputs). From the checkout:

```powershell
$benchDir = Join-Path $env:TEMP ('jecode-bench-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $benchDir | Out-Null
npm run bench:collect -- . "$benchDir\current.json"
npm run bench:compare -- "$benchDir\baseline.json" "$benchDir\current.json" $benchDir
```

Provide a saved compatible `baseline.json` or collect it from a separate clean
checkout with `npm run bench:collect -- <checkout> <new-output.json>`. An optional
last argument chooses 1-5 process repetitions; comparisons require equal counts.
Collection refuses to replace an existing report. A dirty checkout remains
useful raw evidence, but its ratios are withheld because its commit does not
fully identify the measured source. The current collector can measure older
commits while their probe report/workload contracts remain compatible.

`collect.ts` owns execution and checkpointing, `capture.ts` owns bounded child
processes, `collection.ts` owns provenance and validation, `probes.ts` declares
methods/workloads, and `comparison.ts` owns pure comparison and summaries.
`compare.ts` is the bounded file/CLI adapter. All stay outside the npm artifact.
