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

The measured revision deliberately used selective protection for large scopes:

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
latency. That revision imposed a hard archive ceiling of 1 GiB in file lengths
and an active generation ceiling of 512 MiB to leave compaction space. These
ceilings were subsequently replaced by streamed storage and a soft retention
target; see [checkpoint capacity](../usage/CHECKPOINTS.md#scope-policy-and-capacity).

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
a 264 MiB content scope and 50,010 empty files to measure complete acquisition,
points and recovery beyond the former discovery limits. The opt-in `streaming`
workflow captures, prepares, restores, undoes and compacts two unrelated 256 MiB
file states in three fresh archives, verifying exact bytes outside timers.
Its fixtures and comparisons also stream. Fixture creation is timed separately; archive
peak figures are sampled between operations, not continuous allocation peaks.
A copy without Git history can name its revision with
`JECODE_CHECKPOINT_MEASURE_REVISION`.

The journal keeps its followed segment open, and tool writes are recorded
without reading the file back; see
[workspace checkpoints](ARCHITECTURE.md#workspace-checkpoints).

## Scalable checkpoint follow-up

Recorded **2026-10-22**, Rust 1.95.0 release, source fingerprint
`4e3f6ea7dcc110a9a4558fe93dc3cb29 over 409 files`. Windows NTFS fixtures and
controls use warm OS cache. Probes ran serially after compilation. Default
file, discovery and archive admission ceilings are removed; the retention
budget is a soft target. Required current states, pins and recovery survive it.

| Windows operation | Prior baseline median | Current median | Samples |
| --- | ---: | ---: | ---: |
| 4 KiB replacement, paired added cost | 4.446 ms | 2.632 ms | 200 |
| Approximately 1 MiB line edit, paired added cost | 7.651 ms | 4.980 ms | 50 |
| 1 MiB full rewrite, paired added cost | 11.984 ms | 8.384 ms | 50 |
| Unchanged point, 10,000 files | 1.052 ms | 1.152 ms | 50 |
| Restore one changed file | 6.882 ms | 5.746 ms | 50 |
| Restore 100 changed files | 286.151 ms | 306.581 ms | 10 |
| Restore whole scope, 120 changes | 482.230 ms | 567.325 ms | 10 |

The current 1 MiB edit measured 13.392 ms with checkpoints versus 8.568 ms
without them. The paired added range was -64.495 to 100.512 ms. A point
immediately after that edit still waits for exact verification: 177.375 ms
median, 62.152-224.887 ms over 50 samples. Bulk restore medians increased in
these separate runs; the 100-file range was 225.496-677.335 ms. These results
do not isolate causal differences from filesystem variation or establish tail
latency. No antivirus or driver attribution is established.

Full acquisition of 264 MiB took 1,112.645 ms median across three fresh
archives and read all 276,824,064 payload bytes. Full acquisition of 50,010
empty files took 5,591.108 ms across three archives. The earlier selective
measurements copied zero payload and are not comparable full backups.

The streaming probe used two unrelated 256 MiB states and compared exact
bytes after restore, undo and compaction in three fresh archives. Windows
medians were 937.519 ms for acquisition, 1,371.680 ms for durable replacement
preparation, 1,872.448 ms for restore, 1,872.593 ms for undo and 1,753.179 ms
for compaction. The archive held 536,919,409 logical bytes. The entire test
process peaked at 85.20 MiB sampled private memory and 82.01 MiB working set,
sampled every 100 ms; this is not a continuous or whole-system memory peak.

Small revisions retain their previous storage efficiency: 2,138.12 bytes per
40 KiB edit, 41,458 per rewrite and 417 for reused content. One collection
sample reduced 69,643,385 to 3,407,857 logical bytes in 73.961 ms. Physical
allocation differs, and compaction can temporarily retain both generations.

On native WSL/ext4, three fresh archives measured 437.995 ms median for
256 MiB acquisition, 665.283 ms restore and 646.227 ms undo; the streaming test
reported 87.04 MiB maximum RSS. These are current measurements, without a fresh
Linux baseline comparison. Separate offline CLI experiments verified a random
1 GiB + 17-byte file through compaction and reopening, and 100,010 nonempty
files across 101 directories through external changes, restore and undo.
The 1 GiB archive occupied 1,073,837,810 logical / 1,073,934,336 allocated bytes
afterward. Its GC observer sampled 2,147,672,053 logical / 2,147,819,520
allocated bytes at peak, requesting 20 ms between samples. Sampling and `du`
overhead prevent treating this as a continuous maximum or pure GC timing.
Ten alternating CLI samples of 100-file restores measured 249.843 ms median
in one directory and 262.040 ms in 100 directories, including startup and
reconciliation, with exact manifests verified after every operation.

## On-demand checkpoint recovery

Recorded **2026-10-22**, comparing PR head `26d27d8` with `19c67a9` on
Windows 11, NTFS, Rust 1.95.0 `x86_64-pc-windows-gnullvm`, release builds.
The measured source fingerprints are `3bae5a6e03b8563323f446602c870b35 over
413 files` and `21231d80b1c46218e74a7bc9b582be81 over 436 files`.
Each executable was rebuilt separately and identified by SHA-256. Workflows
ran serially without concurrent builds or CI, with warm file caches and
`JECODE_CHECKPOINT_MEASURE_QUICK` unset. These are separate runs, not an
alternating implementation A/B experiment; disk and scheduling variation
remain in the samples.

Normal startup no longer acquires the workspace. File tools save their exact
preimages, and Bash declares protected files or directories. Startup therefore
changes protection coverage; avoiding a full backup is not a faster full-backup
algorithm. [Checkpoint usage](../usage/CHECKPOINTS.md) describes the contract.

| Open before the first tool | Median | Samples | Recorded paths / payload bytes read |
| --- | ---: | ---: | ---: |
| 1,000 source files | 88.116 ms | 10 | 0 / 0 |
| 10,000 source files | 86.502 ms | 5 | 0 / 0 |
| 50,010 empty files | 80.575 ms | 3 | 0 / 0 |
| 264 MiB of untouched content | 95.666 ms | 3 | 0 / 0 |

These fresh archives occupied 1,439-1,440 bytes in file lengths before the first
tool. The 1,000-file range was 84.194-709.818 ms, including the first sample;
the small large-scope samples do not establish tail latency. A foreground Bash
changing one explicitly protected file in the 264 MiB fixture took 155.896 ms
total (30 samples) and read 89 payload bytes, leaving the bulk files uncaptured.
Its single measured restore took 7.069 ms and verified the exact previous bytes.
Background Bash including a 300 ms command delay and checkpoint finalization
took 495.931 ms median over 20 samples.

The explicit full-scan comparison remains available. Initial acquisition of
1,000 files measured 320.888 to 478.252 ms (10 samples; current range
369.577-560.208 ms), while 10,000 files measured 1,416.478 to 1,220.008 ms
(5 samples; 1,026.574-1,353.358 ms). The small full-scan control regressed;
explicit capture is not uniformly faster.
Full acquisition of 264 MiB measured 1,298.020 to 1,204.671 ms, while scanning
50,010 empty files measured 6,117.529 to 8,220.645 ms (three samples each).
Those explicit operations remain proportional to the protected scope.

The following values are median paired added time against the same tool call
without checkpoints. The Bash baseline maintained global observed history;
the new side explicitly acquires declared before-images, so coverage and work
also change.

| Windows tool operation | Prior added | Current added | Samples |
| --- | ---: | ---: | ---: |
| Replace 4 KiB text | 2.915 ms | 2.627 ms | 200 |
| Edit one line of 1 MiB text | 4.166 ms | 4.999 ms | 50 |
| Rewrite 1 MiB text | 9.486 ms | 7.592 ms | 50 |
| Read-only Bash, no declared protection | 32.400 ms | 1.454 ms | 30 |
| Bash changing one declared file | 30.197 ms | 16.188 ms | 30 |
| Bash changing 100 files in a declared directory | 37.314 ms | 101.325 ms | 30 |
| Bash changing 1,000 files in a declared directory | 183.592 ms | 958.733 ms | 10 |

Bulk Bash protection regressed. The 1,000-file added range was
856.238-1,036.939 ms; this is not hidden by the startup improvement. The 4 KiB
paired range was -5.249 to 9.801 ms (p95 6.068 ms). The 1 MiB edit paired range
was -136.695 to 99.054 ms: individual paired differences include substantial
system variation and do not isolate a precise causal overhead. The following
point after a 1 MiB edit still measured 183.955 ms median over 50 samples,
compared with 193.783 ms in the fresh baseline.
For the 1,000-file Bash series, archive counters recorded the same 1,000 file
reads and 2,200 payload bytes per call, but 1,001 to 2,004 journal frames,
176,057 to 287,074 written bytes, and 85.3 to 130 transactions. The 100-file
case similarly changed from 101 to 204 frames. These identify increased archive
work, without assigning all elapsed-time variation to those counters.

An intermediate candidate compacted unheld current heads after ordinary small
writes below budget. Integrated probes exposed a 4 KiB added median of
18.940 ms and a later current-head assertion failure. Collection now waits for
budget pressure, acceptance-collapse, expiry or an explicit request; a
generation-boundary regression test covers that behavior. Durable preparation
and conflict checks were retained.

Recovery also has regressions. These timings include current-state observation,
conflict checking, saving undo and installing the selected state; initial
full-scope preimages are prepared outside the new implementation's timer.

| Recovery operation | Prior median | Current median | Samples |
| --- | ---: | ---: | ---: |
| Restore one changed file | 6.654 ms | 35.959 ms | 50 |
| Restore 100 changed files | 294.524 ms | 353.636 ms | 10 |
| Restore the whole workspace, 120 changes | 528.481 ms | 972.525 ms | 10 |

The one-file current range was 7.526-95.297 ms; the whole-workspace range was
693.129-1,052.196 ms. These are integrated costs, not isolated estimates of a
single storage or path-lookup operation. Saving an undo reference remains part
of each successful restore.

In a 20-revision retention fixture, acceptance reduced median archive file
lengths from 1,732,672 to 87,383 bytes in 77.313 ms; expiry reduced 1,732,650 to
1,824 bytes in 78.212 ms (three samples each). This tests collapsing repeated
versions of one 80 KiB file. It does not imply the same reduction for 20 distinct
files, whose original before-images must all remain until their deadline.

Three fresh-archive streaming samples compared the same 256 MiB workload on
both binaries, with exact-byte verification outside timers:

| Streaming operation | Prior median | Current median |
| --- | ---: | ---: |
| Initial 256 MiB acquisition | 1,007.256 ms | 1,358.292 ms |
| Durable replacement preparation | 1,547.911 ms | 1,523.036 ms |
| Restore | 3,082.436 ms | 3,926.910 ms |
| Undo restore | 3,493.575 ms | 3,068.086 ms |
| Compact two unrelated 256 MiB states | 2,107.179 ms | 3,562.117 ms |

The current restore range was 3,008.510-3,983.883 ms and compaction ranged from
1,830.534 to 3,707.197 ms. Two unrelated states occupied 536,920,649 bytes in
archive file lengths. Small sample counts do not establish tail latency or
remove the large-file restore and collection regressions. File lengths do not
measure allocated disk space or directory metadata.

A separate debug-binary footprint probe protects 20 distinct 80 KiB files in
one group, accepts it, and forces collection. With the 24-hour policy, allocated
archive-file bytes were 1,655,040 before GC, 3,305,736 at the highest sample and
1,655,048 afterward: every distinct original remains available. An immediate
expiry policy (`recovery_accepted_hours: 0`) reduced 1,655,040 to 12,552 allocated bytes;
its highest sample was 1,663,232. All 20 workspace file hashes were unchanged
in both cases. This is not a timed 24-hour wait.

The probe reads `FILE_STANDARD_INFO.EndOfFile` and `AllocationSize` from each
open file handle and includes both generations when observed. It excludes
directory entries, MFT and volume metadata, so it is not a volume free-space
delta. Six/five samples were requested every 20 ms; enumeration adds cost and
can miss shorter peaks. GC wall times with the observer were 172/125 ms and
are not release GC timings. The observed doubling is a real compaction-space
cost even when the payload is still required by an accepted recovery group.

## Measurement conditions

Run from the repository root with Rust 1.95.0. Release builds keep Cargo
compilation outside timed spans. Run probes serially; process/filesystem caches
are not cleared. Each CSV run replaces the corresponding file, so copy results
within `target` before comparing runs.

For platform and physical-terminal checks, see [platform verification](PLATFORMS.md).
