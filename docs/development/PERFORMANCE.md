# Performance

Recorded Windows measurements for UI work, session storage and command startup.
Dates identify the measured implementation; run the probes below to measure the
current source. All probes use synthetic data and make no provider requests.

## TUI responsiveness

Recorded **2026-10-15**, Windows `x86_64-pc-windows-gnullvm`, Rust 1.95.0,
120 x 40 viewport. The baseline is revision `25fb62b`; the optimized column
describes the source measured at that time.

| Operation | Baseline | Optimized |
| --- | ---: | ---: |
| Type at the end of a 256 KiB draft | 57.77–66.61 ms | 0.060–0.093 ms |
| Append formatted text to a 256 KiB response | 19.05–19.28 ms | 0.091–0.122 ms |
| Select a settled tool among 5,000 history items | 69.41–70.52 ms | 0.047–0.062 ms |
| Capture a short draft with 5,000 saved 4 KiB messages | 4.98–6.71 ms | 0.0004–0.0006 ms |
| Scroll three rows in 5,000 cached history items | 0.089–0.132 ms | 0.089–0.124 ms |

Each workload used three warmups and 21 samples, with two runs per variant in
A/B/B/A order. Values span the two run medians. Compilation and terminal writes
were excluded. These measure local UI work, so they do not give keyboard-to-pixel
latency or whole-turn speed. Scrolling showed no improvement beyond run variation.

The changes cached composer layout, reused stable Markdown prefixes, invalidated
only affected tool cards and removed session cloning from idle input capture.
Periodic saves moved disk writes outside the document mutex. A save still clones
its snapshot under that mutex; a very long Markdown source line can exceed a row budget.

Run the current UI probe:

```text
cargo test --locked --offline --release tui::measurement_tests::composer_stream_scroll_and_capture -- --exact --ignored --nocapture --test-threads=1
```

It prints individual samples and summaries in microseconds. Keep other builds
and timing probes out of the run; capture output under ignored `.tmp/` if needed.

## Session history

Recorded **2026-10-01**, Windows `x86_64-pc-windows-gnullvm`, three samples per
size. Alternating user/assistant messages had 128-byte text payloads. Checkpoints
appended batches of 50, including sync and summary-cache writes.

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

At the final 5,000-message sample, working set was 27.6–27.9 MB and private memory
24.1–24.3 MB. The probe holds several history copies in one process; those values
are not the full TUI's memory use or a leak measurement. The workload also leaves
large transcripts, thousands of separate sessions and cold-disk behavior unmeasured.

Run the current storage probe on Windows:

```text
cargo test --locked --offline --release sessions::measurement_tests::session_history_and_listing -- --exact --ignored --nocapture --test-threads=1
```

It writes `windows-sessions.csv` and `windows-sessions-memory.csv` under
`target/measurements/`. Each sample uses a new isolated store. Loading, resume and
context/display reconstruction check the expected complete history.

## Windows process startup

Recorded **2026-10-01** using `hostname.exe`, draining both output streams and
checking stdout and exit status against direct execution.

| Execution path | Samples | Elapsed time |
| --- | ---: | --- |
| Original PowerShell/Add-Type supervisor, warm | 3 | Mean 479.5 ms; range 435.2–553.6 ms |
| Cached compiled supervisor, warm | 9 | Mean 98.8 ms; range 76.1–116.5 ms |
| Compile and run, cold | 3 | Mean 548.9 ms; range 392.2–629.4 ms |
| Direct execution | — | Range 6.7–18.5 ms |

Reusing the supervisor reduced the mean startup cost by about 79% for this short
command. These samples do not establish tail percentiles or the same improvement
for long commands. The cache and fallback are described in
[native boundaries](ARCHITECTURE.md#native-boundaries).

Run the current process probe on Windows:

```text
cargo test --locked --offline --release sessions::measurement_tests::windows_command_supervisor -- --exact --ignored --nocapture --test-threads=1
```

It writes `target/measurements/windows-process.csv`, measuring one initial
supervised launch and then three alternating direct/supervised pairs. A cached
supervisor may already exist, so the initial launch is not necessarily a fresh
compile. Recorded historical results above came from separate comparison runs.

## Measurement conditions

Run from the repository root with Rust 1.95.0. Release builds keep Cargo
compilation outside timed spans. Run probes serially; process/filesystem caches
are not cleared. Each CSV run replaces the corresponding file, so copy results
within `target` before comparing runs.

For platform and physical-terminal checks, see [platform verification](PLATFORMS.md).
