# Performance measurement

Jecode includes opt-in probes for UI work, model-stream parsing, file edits,
session storage, process startup and checkpoints. This page explains how to run
them and records results that still describe the current design. Results were
measured on Windows unless stated; CI on other platforms does not establish
their performance.

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
| `backend::openrouter::stream::tests::tool_argument_stream_bench` | Parsing and assembling a large tool argument split into many stream fragments. |
| `tools::files::streaming::measurements::large_batch_edit_measurement` | A multi-edit batch on a 32 MiB file. |
| `sessions::measurement_tests::session_history_and_listing` | Journal append, load, resume, context projection and listing. Writes CSV files under `target/measurements/`. |
| `sessions::checkpoint_measurements::active_turn_checkpoints` | Conversation saves during an active turn. |
| `sessions::measurement_tests::windows_command_supervisor` | Supervised versus direct process launch on Windows. Writes `target/measurements/windows-process.csv`. |
| `checkpoint_bench::matched_tools` | Complete write, edit, Bash and restore workflows with checkpoints. Configured through the variables below. |
| `tui::measurement_tests::mixed_history_scroll` | Wheel steps over a resumed history of settled turns with prose, code, tables, tool cards with large results and very long answers: the full application step, layout alone, input capture and loop polling. Prints microseconds and VT bytes. |
| `tui::measurement_tests::own_history_scroll` | The same steps over a copy of one saved conversation, in an isolated home. Prints only timings and byte counts. See [Measuring your own conversation](#measuring-your-own-conversation). |
| `tui::terminal::windows::native_tests::native_windows_input_to_vt_latency` | One key and a four-character batch through ConPTY to the TUI's VT output, with and without background checkpoint writes. Each sample follows the previous frame, so ConPTY frame pacing dominates single keys. |
| `tui::terminal::windows::native_tests::native_windows_scroll_latency` | Wheel notches and ten-notch flicks over a resumed mixed history through ConPTY at 80 x 24 and 160 x 50: when the reading counter changes, when output settles, and rows moved per notch. |

`checkpoint_bench::matched_tools` reads `JECODE_BENCH_CASE` (`write4k`,
`edit1m`, `bash1`, `bash100`, `bash1000`, `restore1`, `restore100`,
`restore1000` or `large256m`) and `JECODE_BENCH_COUNT` (measured samples per
block; two warmups precede them). `JECODE_BENCH_ENABLED=0` runs the write, edit
and Bash cases without checkpoint capture as a control. Keep every `BENCH`,
`FOOTPRINT` and `ARCHIVE` row of the output.

The live cache and compaction checks in
[Development and testing](TESTING.md#live-provider-checks) use real accounts and
are not reproducible probes.

## Recorded results

### Subscription context reuse

Recorded 2026-10-31 with bounded live requests, synthetic reference records and
a minimal reply. The warm request appended the previous answer and a new user
message and changed only the request-only session state.

| Route and model | Warm input tokens | Cached tokens | Cached share |
| --- | ---: | ---: | ---: |
| Claude account, Opus 5.5, medium effort | 31,637 | 31,589 | 99.85% |
| ChatGPT account, GPT-6.1 Sol, low effort | 13,280 | 13,056 | 98.31% |

These results show cache reuse across one growing conversation boundary. They
do not measure long-session hit rates, task quality or how a subscription
weights its usage. New tool results, cache expiry and provider routing lower the
share. Single-request latency varied too much to claim a speedup.

A separate live check compacted 20–40 KiB of synthetic task notes with Opus 5.5
at medium effort (about 5 s, 1.1 KB summary), then resumed the saved session and
read the facts back at high effort. The original messages stayed byte-identical.
This does not establish lossless compression for arbitrary histories.

### TUI responsiveness

Recorded 2026-10-15 at a 120 x 40 viewport, comparing revision `25fb62b` with
the optimized source of that date (three warmups, 21 samples, A/B/B/A).

| Operation | Before | After |
| --- | ---: | ---: |
| Type at the end of a 256 KiB draft | 57.8–66.6 ms | 0.06–0.09 ms |
| Append formatted text to a 256 KiB response | 19.1–19.3 ms | 0.09–0.12 ms |
| Select a settled tool among 5,000 history items | 69.4–70.5 ms | 0.05–0.06 ms |
| Capture a short draft with 5,000 saved 4 KiB messages | 5.0–6.7 ms | < 0.001 ms |

The gains come from caching composer layout, reusing stable Markdown prefixes,
invalidating only affected tool cards and saving outside the document lock.
Scrolling showed no change beyond run variation.

On 2026-10-30, expanding tools with Ctrl+O after a 750,000-byte answer took a
longest single layout pass of 4.4–6.1 ms, down from 85–148 ms, because the
rebuild now yields in bounded steps. Total layout work stayed about the same.

### Windows startup, typing and scrolling

The initial implementation was measured on Windows 11 on 2026-11-01, using
release builds under a ConPTY host in A/B/B/A blocks. Reusing the cryptography
helper reduced launch-to-first-frame medians from 948–1,077 ms to 147–148 ms
with one ChatGPT account, one Claude account and an OpenRouter key (10 launches
per block). With an empty adapter cache, medians were 1.35–1.53 s versus
0.44–0.49 s (four launches per block). These startup measurements preceded
the subsequent cancellation and helper-lifetime fixes; they are not final-build
startup timings. An empty configuration showed little change.

A review microbenchmark on the final C# helper repeated the 16-operation
sequence with synthetic files: nine file protections, five hashes and two
directory protections. In A/B/B/A blocks with three warmups and ten samples
each, median times were 1,141.66 / 82.85 / 81.95 / 1,129.45 ms. The pooled
medians were 1,130.63 ms for one process per operation and 82.85 ms for the
reused helper (13.65 times faster for that sequence). The PowerShell driver
included process startup, shutdown and reply checks. It exercised both modes
of the same helper; it did not measure Rust request handling, account-file
reads or the full application launch.

- Account reads protect files and hash identities through the .NET
  cryptography helper. Each operation started a new process (55–80 ms each,
  16 before the first frame); one helper now serves the process, with an idle
  exit and cancellation-aware requests.
- Unbracketed multi-character input still waits for a 50 ms text gap. A 5 ms
  quiet-queue experiment split fragmented file drops and oversized pastes, so
  it was removed. Single keys and completed bracketed pastes remain immediate;
  the experiment's roughly ninefold batch-input speedup does not describe the
  shipped behavior.
- Standard output splits a console write into chunks of a few KB, and ConPTY
  painted a synchronized frame that arrived in several writes at its next
  tick. Frames now use `CONOUT$` to avoid that chunking.
- ConPTY reports a wheel notch as 128 units, so every 15th notch moved six
  rows. Notches are normalized to the native 120-unit scale while preserving
  fine-delta remainders across full notches and direction changes.

Native probes observe ConPTY's VT output, not Windows Terminal's pixels.
Wheel samples use random start offsets to avoid phase lock with frame pacing.
The input probe deliberately sends the next key immediately after the previous
frame, so its single-key timings include ConPTY's inter-frame delay.

Final-code release probes on 2026-11-01 passed with these results (one run,
60 wheel samples per size and 100 input samples per condition):

| Operation | Median | p95 |
| --- | ---: | ---: |
| Wheel notch to changed reading counter, 80 x 24 | 1.58 ms | 2.09 ms |
| Wheel notch to changed reading counter, 160 x 50 | 2.96 ms | 3.98 ms |
| Ten-notch flick to changed reading counter, 80 x 24, from last notch | 9.45 ms | 14.21 ms |
| Ten-notch flick to changed reading counter, 160 x 50, from last notch | 10.82 ms | 13.92 ms |
| Four-character unbracketed batch, idle | 63.18 ms | 65.48 ms |
| One key sent after the preceding frame, idle | 15.08 ms | 15.98 ms |
| Four-character unbracketed batch, checkpoint writes active | 63.39 ms | 64.92 ms |
| One key sent after the preceding frame, checkpoint writes active | 15.04 ms | 15.95 ms |

Every notch moved exactly three rows; every ten-notch flick moved thirty.
The last output byte after a notch arrived at a median 15.42 ms (80 x 24) or
15.76 ms (160 x 50). Inspection of three additional 80 x 24 samples found
only `ESC[m` style resets after the counter update, with no further cell text.
Do not interpret first-counter timings as time to a completely quiet stream.

For 1,000 settled mixed turns, Jecode's own wheel step took a median 0.15 ms
at 80 x 24 and 0.56 ms at 160 x 50 (21 samples; maxima 0.29 and 1.11 ms).
Settled event-loop polling took 0.2–0.3 microseconds at the median.

### Measuring your own conversation

`own_history_scroll` scrolls a copy of one saved conversation, so a slow
history can be measured without sharing it. It copies the journal into a
temporary home and accepts only settled conversations with no active turn,
pending question or queued input. Collaborator auto-resume is disabled for
the measurement. It prints only timings and byte counts; it does not continue
agent work or call a provider:

```text
JECODE_MEASURE_JOURNAL=<path to a session .jsonl>
JECODE_MEASURE_DIRECTORY=<the folder the session belongs to>
cargo test --locked --offline --release tui::measurement_tests::own_history_scroll -- --exact --ignored --nocapture --test-threads=1
```

Journals are under the Jecode home's `sessions/` directory; the folder is the
one where the conversation was started.

### Model stream and file edits

Recorded 2026-10-24. Parsing a 256 KiB tool argument delivered in 4,096 stream
fragments takes about 6 ms, down from 399–467 ms at `674eba6`.

A batch of ten edits on a 32 MiB file now composes the replacements and writes
the final content once: the pooled median fell from 778 ms to 327 ms against
`97b4284`, with checkpoints disabled and warm caches. Matching still scans the
file once per edit.

### Session history

Recorded 2026-10-01 with 128-byte messages, three samples per size.

| Operation | 1,000 messages | 5,000 messages |
| --- | ---: | ---: |
| Load journal | 1.3–1.5 ms | 5.7–10.5 ms |
| Resume and save | 6.7–11.2 ms | 22.4–25.9 ms |
| Project active context | 0.4–0.5 ms | 1.9–2.1 ms |
| List one cached session | 0.3–0.5 ms | 0.3–0.4 ms |
| List one uncached session | 5.7–11.0 ms | 8.7–12.8 ms |
| Journal size | 169 kB | 845 kB |

### Checkpoints

Recorded 2026-10-22 on NTFS with release builds, comparing the per-call engine
(`76b214e`) with the earlier history-keeping engine (`6679d6c`).

| Operation | Earlier median | Per-call median |
| --- | ---: | ---: |
| Write 4 KiB | 3.89 ms | 4.85 ms |
| Edit 1 MiB | 34.73 ms | 33.22 ms |
| Bash, protect 1,000 files | 712.88 ms | 382.27 ms |
| Restore 1 file | 7.57 ms | 18.11 ms |
| Restore 1,000 files | 3,310.25 ms | 2,944.20 ms |
| Edit 256 MiB | 1,473.01 ms | 1,269.55 ms |

Small operations show no consistent speedup, and single-file restore is slower.
The per-call engine stores an independent full copy for every call and does not
deduplicate: in a highly repetitive fixture, five 256 MiB edits used 1,280 MiB
of archive against 3 MiB for the earlier engine. The 24-hour expiry bounds how
long packs are kept, not how much space they use; see
[Checkpoints](../usage/CHECKPOINTS.md#storage-and-expiry).
