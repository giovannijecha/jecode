# Manual performance probes

These five probes exercise distinct production paths with synthetic fixtures.
Run them from the repository root, one at a time on an otherwise idle machine:

```powershell
npm run --silent bench:context
npm run --silent bench:redaction
npm run --silent bench:search
npm run --silent bench:session
npm run --silent bench:transcript
```

Use a supported Node.js version and record the commit being measured. `--silent`
removes npm's command banner so stdout contains one JSON report. Each report has
the same envelope:

```json
{
  "benchmark": "workspace-search",
  "environment": {
    "node": "v24.0.0",
    "platform": "win32",
    "arch": "x64"
  },
  "results": {}
}
```

The environment values come from the running process; the example is only the
report shape. Results contain numeric measurements in milliseconds, scenario
sizes, and iteration counts. Measurements are rounded to three decimal places
for reporting; thresholds use the original values. The context and session
reports also contain thresholds and `passed`. A threshold failure returns a
nonzero exit code after emitting its report. The other probes report timings
without a performance pass/fail decision; fixture correctness failures still
fail the command.

| Probe | Scenario and interpretation |
| --- | --- |
| `context.ts` | Estimate one request just below 8 MiB and force a compaction plan for 128 messages totaling 4 MiB. One warm-up, then five measurements. Each operation must have a median duration at most 2,000 ms and a median of per-run maximum event-loop stalls at most 75 ms. |
| `redaction.ts` | Redact one 30,000-character non-matching chunk against the maximum supported number of synthetic secrets. One warm-up, then five measurements; report the median. |
| `search.ts` | Search 600 temporary 4 KiB files for an absent string. One warm-up, then five measurements; report the median. |
| `session.ts` | Compare checkpoints at 50/750 nodes, catalogues of 12 sessions at 1/200 nodes each, and loads at 1/200/750/1,024 nodes. Warm each scenario first; measure five checkpoint/catalogue runs and three load runs. The report includes all size, duration, scaling, and depth-delta thresholds. |
| `transcript.ts` | Render 20,000 answers plus a live reasoning block starting at 200,000 characters. Measure first viewport and background reflow at 100/80/120 columns, 100 cached resizes, 500 stable frames, 200 streaming frames, sealing, and 200 expanded live frames. These are sequential phases on one renderer, not independent median samples. |

The session thresholds remain: a large-checkpoint median at most 25 ms and scale
at most 2; a deep-catalogue median at most 20 ms and shallow-to-deep increase at
most 10 ms; a 1,024-node load median at most 350 ms and scale from 200 nodes at
most 7. These and the context limits are manual regression tripwires, not CI
gates or hardware-independent guarantees.

Compare repeated runs before and after a change using the same machine, Node.js
version, power settings, storage, and scenario. Look at scaling and individual
phases as well as total time. A threshold failure warrants another idle run and
investigation; changing a limit requires a separate rationale. Keep any saved
reports under the repository's ignored `sandbox/` directory rather than tracking
machine-specific timing baselines here.

Search and session probes create and clean temporary data. Redaction uses
synthetic secrets and points credential stores at an absent temporary path.
The probes import production implementations directly; no provider requests or
second implementation of the measured behavior belongs here. `report.ts` only
provides the common JSON envelope and numeric formatting.
