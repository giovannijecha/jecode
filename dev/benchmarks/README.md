# Manual performance probes

These six probes exercise distinct production paths with synthetic fixtures.
Run them from the repository root, one at a time on an otherwise idle machine:

```powershell
npm run --silent bench:context
npm run --silent bench:redaction
npm run --silent bench:search
npm run --silent bench:session
npm run --silent bench:transcript
npm run --silent bench:tui
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
| `context.ts` | Estimate one request just below 8 MiB and force a compaction plan for 128 messages totaling 4 MiB. One warm-up, then five measurements; each operation must have a median duration at most 2,000 ms and median maximum event-loop stall at most 75 ms. Then run 12/40 inert 16 KiB reads through the controller and context manager: require zero summaries for the short case and one or two for the long case, while preserving complete canonical evidence. |
| `redaction.ts` | Redact one 30,000-character non-matching chunk against the maximum supported number of synthetic secrets. One warm-up, then five measurements; report the median. |
| `search.ts` | Search 600 temporary 4 KiB files for an absent string. One warm-up, then five measurements; report the median. |
| `session.ts` | Compare checkpoints at 50/750 nodes, catalogues of 12 sessions at 1/200 nodes each, and loads at 1/200/750/1,024 nodes. Warm each scenario first; measure five checkpoint/catalogue runs and three load runs. The report includes all size, duration, scaling, and depth-delta thresholds. |
| `transcript.ts` | Render 20,000 answers plus a live reasoning block starting at 200,000 characters. Measure first viewport and background reflow at 100/80/120 columns, 100 cached resizes, 500 stable frames, 200 streaming frames, sealing, and 200 expanded live frames. These are sequential phases on one renderer, not independent median samples. |
| `tui.ts` | Run the production input loop, frame scheduler, composer, menus, transcript, and differential painter. Compare short/2,000-block histories, 60/120 columns, and a throttled output consumer. Report latency distributions, output pressure, and memory around work/reset/close. See the method below. |

The session thresholds remain: a large-checkpoint median at most 25 ms and scale
at most 2; a deep-catalogue median at most 20 ms and shallow-to-deep increase at
most 10 ms; a 1,024-node load median at most 350 ms and scale from 200 nodes at
most 7. These and the context limits are manual regression tripwires, not CI
gates or hardware-independent guarantees.

Candidate comparisons and the limits of these synthetic measurements are
described in the [validation protocol](../validation/README.md#performance).

Compare repeated runs before and after a change using the same machine, Node.js
version, power settings, storage, and scenario. Look at scaling and individual
phases as well as total time. A threshold failure warrants another idle run and
investigation; changing a limit requires a separate rationale. Saved reports
follow the shared [temporary-output lifecycle](../README.md#temporary-outputs).

Search and session probes create and clean temporary data. Redaction uses
synthetic secrets and points credential stores at an absent temporary path.
The probes import production implementations directly; no provider requests or
second implementation of the measured behavior belongs here. `report.ts` only
provides the common JSON envelope and numeric formatting.

## Integrated TUI

`bench:tui` enables `--expose-gc` for memory observations and runs four cases in
order: an empty history at 120 columns, 2,000 blocks at 120 columns, the same
history at 60 columns, and 120 columns with a 32 KiB/s asynchronous consumer.
All frames have 40 rows. The larger fixture includes 2,000 lines of collapsed
tool evidence. Each case starts one inert provider request, emits 212,500
characters of reasoning, streams answer chunks, interrupts, and resets via
`/new`. No provider request reaches the network and no command tool executes.

After 25 composer warm-up actions, each case records 100 observations for
typing, typing during streaming, menu selection during streaming, and resize
during streaming. Input sequences replace a short draft through real key
decoding. Each observation waits for a frame showing its own change, rather
than accepting the next unrelated frame. These are synthetic workflows, not
recordings of human typing. A failed scenario exits nonzero; timings have no
new pass/fail threshold.

The report separates these measurements:

- `inputToFrameMilliseconds`: input dispatch through the normal frame timer,
  composition, matching-frame observation, and the production painter.
- `inputToWriteMilliseconds`: the same start through an actual output write.
  Under pressure, multiple composed states can be replaced by the latest frame.
- `inputToAcknowledgementMilliseconds`: the same start through the consumer's
  write callback. The slow case includes queued output; the next input does not
  wait for acknowledgement.
- `painterMilliseconds` and `bytesPerWrittenFrame`: differential encoding/write
  time and UTF-8 bytes. `written` and `coalesced` count observed states that reached
  output or were superseded. Write/acknowledgement distributions include only
  written observations and are null when none were written. p95 uses nearest rank.

Output totals include background and warm-up frames. Maximum queued bytes and
writes returning false expose pressure. The host keeps only the latest frame,
one pending output observation, and numeric samples; it does not store a video.
Memory fields are process-wide bytes after explicit garbage collection before
the case, after warm-up, after work, after `/new`, and after shutdown/output drain.
Snapshots include fixtures and measurement machinery. Module warm-up, allocator
retention, and pending output can affect RSS; compare equivalent phases and
repeated runs before diagnosing a leak. Collection occurs between phases, not
inside timed input actions.

Colour follows the host terminal and `NO_COLOR`; the report records the actual
mode. Motion is reduced for repeatability. The throttled `Writable` models an
asynchronous consumer; it does not model a blocked synchronous system call.
Actual stdout behavior varies by platform and destination; see Node's
[process I/O contract](https://nodejs.org/docs/latest-v24.x/api/process.html#a-note-on-process-io).
Neither write completion nor acknowledgement measures terminal painting or
physical key-to-display latency. Those remain real-terminal validation work.

`tui-scenario.ts` owns fixtures and the sequence, `tui-host.ts` observes the
production app, and `tui-output.ts` counts and throttles output. These helpers and
their tests stay out of the release package. Run on an otherwise idle machine;
the throttled case takes longer while its output drains.
