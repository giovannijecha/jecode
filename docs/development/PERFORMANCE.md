# Performance measurement

Jecode includes opt-in probes for UI work, model-stream parsing, file edits,
session storage and process startup. This page explains how to run the probes
and interpret their scope. The latest recorded measurements are in
[Historical performance results](../reports/PERFORMANCE_RESULTS.md); each
section identifies its measured date and host.

For comparisons with other coding agents, see the
[historical Jecode vs Codex and Claude Code benchmark](../reports/AGENT_BENCHMARK_2026-11.md). It
records coding work with earlier Jecode account routes; the probes below measure
separate parts of Jecode and do not establish a lead over another harness.

## Measurement rules

- Build before timing and use release builds (`--release`) unless a probe says
  otherwise.
- Run variants serially with no builds, CI jobs or other probes in progress.
- Compare versions in A/B/B/A order, with warmups before each block.
- Keep fixture setup, resets and output verification outside the timed interval.
- Keep captured output in the ignored `.tmp/` directory.
- Report what a probe measures. Local probes measure Jecode's own work; they do
  not establish keyboard-to-pixel latency, provider latency or whole-turn speed.

## Probes

Run each probe with
`cargo test --locked --offline --release <name> -- --exact --ignored --nocapture --test-threads=1`.

| Probe | Measures |
| --- | --- |
| `tui::measurement_tests::composer_stream_scroll_and_capture` | Typing in a large draft, appending formatted text, selecting and scrolling large histories, capturing input. Prints samples in microseconds. |
| `tui::transcript::rebuild_tests::long_answer_tool_toggle_measurement` | Ctrl+O layout work after a very long answer. |
| `tui::render::tests::active_tests::active_long_history_frames` | Frames of a 200- and 1000-turn history while it keeps streaming, finishing tool calls, scrolling and resizing: frame time percentiles, frames with layout still pending, and how many frames each new stream needs to become visible. |
| `backend::openrouter::stream::tests::tool_argument_stream_bench` | Parsing and assembling a large tool argument split into many stream fragments. |
| `tools::files::streaming::measurements::large_batch_edit_measurement` | A multi-edit batch on a 32 MiB file. |
| `sessions::measurement_tests::session_history_and_listing` | Journal append, load, resume, context projection and listing. Writes CSV files under `target/measurements/`. |
| `sessions::measurement_tests::windows_command_supervisor` | Supervised versus direct process launch on Windows. Writes `target/measurements/windows-process.csv`. |
| `tui::measurement_tests::mixed_history_scroll` | Wheel steps over a resumed history of settled turns with prose, code, tables, tool cards with large results and very long answers: the full application step, layout alone, input capture and loop polling. Prints microseconds and VT bytes. |
| `tui::measurement_tests::own_history_scroll` | The same steps over a copy of one saved conversation, in an isolated home. Prints only timings and byte counts. See [Measuring your own conversation](#measuring-your-own-conversation). |
| `tui::terminal::windows::native_tests::native_windows_input_to_vt_latency` | One key and a four-character batch through ConPTY to the TUI's VT output. Each sample follows the previous frame, so ConPTY frame pacing dominates single keys. |
| `tui::terminal::windows::native_tests::native_windows_scroll_latency` | Wheel notches and ten-notch flicks over a resumed mixed history through ConPTY at 80 x 24 and 160 x 50: when the reading counter changes, when output settles, and rows moved per notch. |

The live cache and compaction checks in
[Development and testing](TESTING.md#live-provider-checks) use real accounts and
are not reproducible probes.

## Measuring your own conversation

`own_history_scroll` scrolls a copy of one saved conversation, so a slow
history can be measured without sharing it. It copies the journal into a
temporary home and accepts only settled conversations with no active turn,
pending question or queued input. Collaborator auto-resume is disabled for
the measurement. It prints only timings and byte counts; it does not continue
agent work or call a provider. On PowerShell:

```powershell
$env:JECODE_MEASURE_JOURNAL = 'C:\path\to\session.jsonl'
$env:JECODE_MEASURE_DIRECTORY = 'C:\path\to\project'
cargo test --locked --offline --release tui::measurement_tests::own_history_scroll -- --exact --ignored --nocapture --test-threads=1
Remove-Item Env:JECODE_MEASURE_JOURNAL, Env:JECODE_MEASURE_DIRECTORY
```

Journals are at `sessions/<project>/<session>/session.jsonl` in the Jecode
home (see [Files Jecode keeps](../reference/FILES.md)); the session folder's
name identifies the conversation, and
`JECODE_MEASURE_DIRECTORY` is the folder where it was started.
