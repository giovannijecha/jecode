# Performance

Recorded measurements for UI work, session storage, command startup and
workspace checkpoints, on Windows unless stated.
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

## Per-call recovery

Recorded **2026-10-22** on Windows 11 build 26300, NTFS, Intel Core Ultra 9
275HX (24 logical processors), Rust 1.95.0 `x86_64-pc-windows-gnullvm`, release
builds. The baseline is the previous PR head `6679d6c`; the candidate is the
independent per-call engine documented in [Checkpoints](../usage/CHECKPOINTS.md).
These compare complete tool workflows with different retention contracts, not
an isolated storage optimization: the baseline maintains history and after
states; the candidate saves only preimages for a fixed 24 hours.

| Operation | Samples per variant | Baseline median | Per-call median | Median change |
| --- | ---: | ---: | ---: | ---: |
| Write 4 KiB | 60 | 3.89 ms | 4.85 ms | +24.5% |
| Edit 1 MiB | 40 | 34.73 ms | 33.22 ms | -4.4% |
| Bash, protect 1 file | 20 | 142.01 ms | 135.73 ms | -4.4% |
| Bash, protect 100 files | 20 | 210.58 ms | 175.70 ms | -16.6% |
| Bash, protect 1,000 files | 12 | 712.88 ms | 382.27 ms | -46.4% |
| Restore 1 file | 60 | 7.57 ms | 18.11 ms | +139.3% |
| Restore 100 files | 20 | 362.99 ms | 335.21 ms | -7.7% |
| Restore 1,000 files | 12 | 3,310.25 ms | 2,944.20 ms | -11.1% |
| Edit 256 MiB | 6 | 1,473.01 ms | 1,269.55 ms | -13.8% |

Small operations show no consistent speedup. Single-file restore remains slower
than the baseline, even after narrower path leases and avoiding a final reread
reduced its earlier observed median of 46.54 ms. The modest differences on short
calls and bulk restore overlap observed variation and do not establish a general
speedup.
Bash includes native process startup; it runs `:` and protects its declared
files. There is no post-command scan in the candidate.

Observed sample ranges, including outliers:

| Operation | Baseline range | Per-call range |
| --- | ---: | ---: |
| Write 4 KiB | 3.57-12.30 ms | 4.45-7.31 ms |
| Edit 1 MiB | 25.61-43.24 ms | 25.66-338.74 ms |
| Bash, 1 file | 90.92-428.41 ms | 129.57-146.38 ms |
| Bash, 100 files | 197.57-269.64 ms | 124.00-371.85 ms |
| Bash, 1,000 files | 416.84-1,199.63 ms | 221.12-1,904.70 ms |
| Restore 1 file | 4.71-93.59 ms | 8.42-128.51 ms |
| Restore 100 files | 169.53-1,096.59 ms | 242.74-887.47 ms |
| Restore 1,000 files | 2,395.40-3,788.37 ms | 2,457.29-3,999.76 ms |
| Edit 256 MiB | 1,207.47-1,600.75 ms | 1,027.11-1,548.48 ms |

### Method

Both versions use the same opt-in `checkpoint_bench::matched_tools` workload.
The baseline changes only the session setup arity; the shared restore adapter
selects its old `restore_all` action for a whole captured scope. Each case runs
in A/B/B/A order in separate processes, with two warmups before each block.
Reported medians pool both blocks for each variant and average the middle two
samples when the count is even. Builds and CI did not run during timing.

Fixtures and resets are outside the timer. Ordinary files are 4 KiB; the 1 MiB
edit replaces one unique five-byte prefix. Restore acquires the same protection
scope before **every** sample in both versions, outside the timer, then changes
the files directly. Every workload checks exact resulting bytes outside the
timed interval. The 256 MiB fixture is generated and verified with 64 KiB buffers,
without allocating a file-sized fixture. Tests use isolated homes, no provider,
and no maintenance worker. Helpers and filesystem caches are warm; cold native
helper compilation, cold-disk behavior and full TUI latency are not measured.
No tail percentile or general throughput claim follows from these sample sizes.

### Memory and disk tradeoff

Peak main-process working set, sampled every 20 ms over each whole workload,
was 29.4 -> 8.8 MiB for 1,000-file restore and 13.0 -> 6.8 MiB for the 256 MiB
edit. These exclude child processes and the OS page cache; sampling can miss
shorter peaks. Payload buffering stays bounded, while metadata and traversal
memory still grow with the selected path count.

Independent packs remove global replay, after-state capture, shared object
bookkeeping and copying compaction. They deliberately give up deduplication.
Archive **file lengths**, including warmups, illustrate the cost:

| Repeated calls in one block | Baseline archive | Per-call archive |
| --- | ---: | ---: |
| 32 writes of 4 KiB | 36.9 KiB | 134.5 KiB |
| 22 edits of 1 MiB | 1.02 MiB | 22.00 MiB |
| 5 edits of 256 MiB | 3.03 MiB | 1,280.00 MiB |

The fixtures are highly repetitive: one prefix followed by repeated `x` bytes.
The old store reuses identical chunks within and across files, giving it an
especially large advantage in the last row. These are not typical source-tree
space ratios, filesystem allocated bytes or a claim that all archives shrink.
The new engine stores a complete independent preimage for each call. A 24-hour
TTL bounds retention time, not bytes; the advisory 256 MiB free-space margin
can result in partial coverage. Restore additionally needs its undo and staged
replacement space. Expiry unlinks individual packs without a full archive copy;
physical deletion waits for maintenance or the next launch and active readers.

The checkpoint subtree changed from 107 files / 28,106 lines at `6679d6c` to
31 files / 7,322 lines, including its tests. Startup and new turns perform no
workspace acquisition; this is covered by behavioral tests rather than a
startup timing claim. The merged base `2239186` had 68 files / 18,242 lines in
that subtree, so the replacement is about 60% smaller than the base too.

### Reproduce

Build before timing, using the platform setup in [Testing](TESTING.md). Set
`JECODE_BENCH_CASE` to `write4k`, `edit1m`, `bash1`, `bash100`, `bash1000`,
`restore1`, `restore100`, `restore1000` or `large256m`; set `JECODE_BENCH_COUNT`
to the measured samples per block (30, 20, 10, 10, 6, 30, 10, 6, 3 respectively).
Then run:

```text
cargo test --locked --offline --release checkpoint_bench::matched_tools -- --exact --ignored --nocapture --test-threads=1
```

`JECODE_BENCH_ENABLED=0` provides a write/edit/Bash control without recovery.
For comparisons, build both binaries first and invoke their test executables
serially in A/B/B/A order. Retain every `BENCH`, `FOOTPRINT` and `ARCHIVE` row;
measure process working set separately. Do not compile or run other probes at
the same time. Wall-clock expiry tests advance injected timestamps instead of
waiting 24 hours. Performance figures above were measured on Windows; CI tests
on other platforms do not establish their performance.
