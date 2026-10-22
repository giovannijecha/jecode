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

## Workspace checkpoints

Recorded **2026-10-21**, source fingerprint
`8bd94daa978e2b964bc67f58277ff822 over 384 files`, with Rust 1.95.0 release
builds and a warm OS cache. Windows uses `x86_64-pc-windows-gnullvm`, NTFS and
real-time antivirus; Linux uses WSL2 and native ext4. Added time is the median
paired difference against the same call without checkpoints; other values are
medians. Parentheses give the actual sample counts.

| Operation | Windows | Linux |
| --- | ---: | ---: |
| `write` 4 KiB, added | 4.834 ms (200) | 5.314 ms (20) |
| `edit` one line of 1 MiB text, added | 11.637 ms (50) | 2.941 ms (5) |
| Bash command changing one file, added | 30.593 ms (30) | 15.735 ms (3) |
| Reused point, 1,000 unchanged files | 0.788 ms (200) | 2.804 ms (200) |
| Reused point, 10,000 unchanged files | 1.017 ms (50) | 18.446 ms (50) |
| Point right after a 1 MiB edit | 214.645 ms (50) | 13.902 ms (5) |
| Initial capture, 10,000 files | 886.654 ms (5) | 201.098 ms (5) |
| Restore one / 100 files | 8.424 / 265.021 ms (50 / 10) | 10.793 / 241.205 ms (50 / 10) |
| Four processes, shared paths, per write | 9.263 ms (400) | 14.475 ms (400) |

Linux tool-call measurements used the quick workload; the small samples do not
establish tail percentiles or a speedup over prior measurements. Capture,
ordinary points, recovery, storage and sessions used full sample counts on
both platforms. The 1,000-file point p95 was 0.992 ms on Windows and 3.258 ms
on Linux. Four shared-path processes had p95 54.392 / 25.522 ms respectively;
every variant reported zero failed or unprotected writes and verified all final
files byte for byte. The Windows maximum was 499.931 ms.

A fresh run of starting source fingerprint `d67d22b744038f484770ba000c041244`
measured 11.544 ms for the Windows
1,000-file reused point: the final median is 14.6 times faster. Ordinary native
points drain delivered notifications without a quiet wait; they are observed
history, not atomic filesystem snapshots. Windows polling fallback measured
11.496 ms for 1,000 files and 53.666 ms for 10,000 files. Small writes and
concurrent writes cost more than in the earlier source without hard admission;
the point speedup does not apply to every operation.

A point right after a large write waits for the write's verification reread.
An integrated trace placed about 199 ms inside `File::open` on the recently
edited Windows source file. Real-time antivirus was enabled, but its
contribution was not isolated from other filesystem or scheduling delays.
The exact reread remains.

Large scopes deliberately use selective protection:

| Workload | Windows | Linux |
| --- | ---: | ---: |
| 264 MiB eligible content, readiness | 73.618 ms (3) | 15.791 ms (3) |
| Reused point with two protected work files in that scope | 0.727 ms (200) | 0.372 ms (200) |
| 50,010 empty files, bounded discovery until readiness | 1,416.317 ms (3) | 842.909 ms (3) |

These baselines copied zero file payload bytes and reported selective coverage;
they do not measure a complete 264 MiB backup. The Windows content startup
ranged from 72.428 to 625.138 ms, including the first cold helper. Fixture
creation is outside readiness timers. Restoring the two protected files verified
their exact bytes and left an eligible 8 MiB sentinel untouched. The sampled
archive peak after those operations was 4,876 bytes on Windows and 4,932 on
Linux; sampling does not establish a continuous allocation peak.

Collection reduced a long turn's 69.6 MB archive to 3.4 MB: 95.1% released,
in one 80.809 ms Windows sample and one 41.453 ms Linux sample. It retained
current states and the last named point. These samples do not establish GC tail
latency. The hard archive ceiling is 1 GiB in file lengths, with an active
generation ceiling of 512 MiB to leave compaction space.

### Explicit capture follow-up

Recorded **2026-10-22**, Rust 1.95.0 release builds, source fingerprint
`aed942d290528925d84a579be98bb33e over 388 files`. The offline workspace API
fixture uses an already open archive, a reduced 128 MiB budget and selective
coverage. Files are prepared before timing and remain in the OS cache. First
capture stores distinct, previously unrecorded contents; repeated capture
reuses an existing state. Process/archive startup is excluded.

| Capture | Windows median / p95 | Linux median / p95 | Samples |
| --- | ---: | ---: | ---: |
| First protection, 4 KiB | 2.898 / 3.354 ms | 4.968 / 5.888 ms | 200 |
| First protection, 1 MiB | 8.259 ms | 12.360 ms | 20 |
| Repeated state, 4 KiB | 1.757 / 2.075 ms | 2.422 / 2.888 ms | 200 |
| Repeated state, 1 MiB | 4.359 ms | 3.220 ms | 20 |

The opt-in measurement checks captured bytes outside timed spans and exercises
capture → Bash → restore:

```text
cargo test --locked --offline --release checkpoints::targeted::tests::measure_explicit_capture_in_selective_workspace -- --exact --ignored --nocapture --test-threads=1
```

Native Windows ConPTY input-to-VT measurements took 100 samples per mode.
Single-key median/p95 was 12.258/20.208 ms idle and 12.662/20.652 ms while
checkpoint writes progressed. Four-character bursts measured about 58/70 ms
because the existing Windows paste-coalescing interval applies. This measures
VT output, not physical keyboard-to-pixel latency. Rust fixture child peak RSS
was 8.24 MiB idle and 16.60 MiB active; helper processes are excluded.

Three Windows baseline/final comparisons with four shared-path processes had
p95 values 58.396/64.792/68.237 ms before and 57.897/57.347/57.602 ms after.
Only notification capture batches now limit simultaneous lease stripes;
full scans keep their prior batching. Throughput varies and outliers remain.
The final session harness additionally compares decoded archive objects with
filesystem bytes; 1,400 mutations per platform reported no failed or
unprotected writes. The integrated timings and TUI measurements used source
`8da1d08577c8c4638ce6bb532e922e02`; the final fingerprint changes only ignored
measurement fixtures, and the release product binary is identical.

A guided live-provider check using the configured model completed restore/undo,
selective capture/Bash/recovery and conflict preservation in 3/3 cases. It is
an explicitly opted-in test, not a general reliability estimate or an offline
test. Native macOS execution remains unverified.

Run the integrated checkpoint workflows against the optimized build:

```text
cargo test --locked --offline --release checkpoints::measurement_tests::workflows -- --exact --ignored --nocapture --test-threads=1
```

The run covers initial capture and reopen, points, tool calls and Bash with and
without checkpoints, history reads, restores, storage growth and collection,
and concurrent sessions as separate processes. Tool comparisons use two
identical synthetic workspaces in alternating order after two untimed warm-up
pairs, and every file-tool call writes content the archive does not hold yet;
the storage workflow measures reused content separately. File-tool pairs
follow an untimed pause, as model latency separates real calls; one series
edits twice in a row without it. Results go to
`.tmp/checkpoint-measurements/<os>/` as `samples.csv`, `summary.csv`,
`metrics.csv` and `environment.json`; set
`JECODE_CHECKPOINT_MEASURE_OUT` to choose another directory, or
`JECODE_CHECKPOINT_MEASURE_QUICK=1` to take a tenth of the samples while
changing the harness. To repeat selected workflows, set
`JECODE_CHECKPOINT_MEASURE_ONLY` to comma-separated names: `capture`, `editing`,
`recovery`, `storage`, `sessions`. The additional opt-in `large` workflow creates
a 264 MiB content scope and 50,010 empty files to measure bounded discovery,
selective points and recovery. Fixture creation is timed separately; archive
peak figures are sampled between operations, not continuous allocation peaks.
A copy without Git history can name its revision with
`JECODE_CHECKPOINT_MEASURE_REVISION`.

The journal keeps its followed segment open, and tool writes are recorded
without reading the file back; see
[workspace checkpoints](ARCHITECTURE.md#workspace-checkpoints).

## Measurement conditions

Run from the repository root with Rust 1.95.0. Release builds keep Cargo
compilation outside timed spans. Run probes serially; process/filesystem caches
are not cleared. Each CSV run replaces the corresponding file, so copy results
within `target` before comparing runs.

For platform and physical-terminal checks, see [platform verification](PLATFORMS.md).
