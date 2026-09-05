# Record context behavior

Use the production TUI with an opt-in diagnostic subscriber when investigating
context pressure, unexpected compaction, or differences between estimates and
provider-reported counts. This development launcher is not part of the npm CLI.

From the workspace being tested, invoke the checkout's launcher. Normal launch
arguments still apply; `-c` resumes the latest conversation in that workspace:

```powershell
node C:\Users\giova\Codex\jecode\dev\context\record.ts -c
```

The command does not create a synthetic conversation or select a provider. It
starts the regular interactive path with the same settings and permissions.
Record the checkout commit and provider/model separately. An ordinary `jecode`
launch records nothing. Use an isolated development `JECODE_HOME` for destructive
recovery tests as required by the [validation protocol](../validation/README.md).

## Evidence and bounds

Each invocation exclusively creates one `context-*.jsonl` file under
`~/.jecode/diagnostics/` (or the isolated development home). The path and recording
totals are printed after the terminal is restored. Files can be inspected during
the run; ignore an incomplete last line after an abrupt stop. Normal close drains
pending writes and appends an `end` record.

- Request records contain local estimate, chosen input budget, calibration source,
  and returned provider input tokens when present. Cached tokens remain part of
  input usage; these records are not billing calculations.
- Compaction records contain budget/overflow/manual cause, outcome, before/after
  counts when available, and total local-plus-provider duration. `no-prefix` means
  planning found no eligible prefix; `cancelled` and `timeout` remain distinct.
  A zero `beforeTokens` means an internal caller did not supply a measurement.
- Records contain no prompts, summaries, file paths, tool arguments, output,
  credentials, account IDs, provider raw data, or arbitrary error messages.
- Each file accepts at most 4,096 events plus the end record. The asynchronous
  writer queues at most 64 pending events. Excess events are counted as dropped;
  they never delay model/tool execution. A dropped record prevents a claim that
  the trace is complete. I/O failure stops recording and is reported on close.
- Files use exclusive creation and owner-only permissions where supported; the
  output directory must be a direct, anchored directory. The channel subscriber
  whitelists fields even when a publisher supplies additional properties.

An accepted summary is not proof of a committed checkpoint. Session validation
is still needed to confirm the durable anchor. The trace also does not establish
summary quality, display latency, or accessibility. Capture those observations
using the release-candidate protocol.

Keep recordings local and outside version control. Retain the sanitized evidence
needed for review, then remove obsolete recordings through the
[temporary-output lifecycle](../README.md#temporary-outputs).
