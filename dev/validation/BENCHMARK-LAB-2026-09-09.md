# Benchmark laboratory validation, 2026-09-09

This is development evidence for the benchmark-maintenance working tree based
on `769f47bb41bbf921a6f7ea70716851c9df461770`. It is not a release-candidate
acceptance record or a new live Codex/Jecode comparison. No provider account,
external generation, or real user session was used. Production `src/` remains
unchanged: the measured optimization is in the comparison laboratory.

## Environment and method

- Windows x64, Node 24.18.0: typecheck, clean-build tests, full repository gates.
- Native WSL2 Linux x64, kernel `6.18.33.2-microsoft-standard-WSL2`, Node 24.20.0,
  Python 3.14.4, Intel Core Ultra 9 275HX, 24 visible CPUs, approximately 15.3 GiB
  guest memory. Sources, fixtures and results were on the native Linux filesystem.
- Serial execution; no heavy suite overlapped a benchmark. The six-probe
  acquisition used one process per probe on the clean base, then the working
  candidate. This is acquisition screening, not an ABBA product comparison.
- The new collector acquired the old entry points despite newly introduced
  helper files being absent. Both collections completed with all six probes
  successful. Candidate evidence remained explicitly dirty; all six comparative
  ratios were withheld. Old/new methods are not interchangeable baselines.
- Raw collections were 31,992 and 46,064 bytes, below the existing report bounds.
  Sanitized observations are retained below; disposable logs and workspaces are
  removed after validation under the development temporary-output policy.

## Correctness and repository checks

| Check | Observed result |
| --- | --- |
| Windows `npm run typecheck` | Pass. |
| Windows `npm test` | 1,099 tests: 1,088 pass, 11 platform skips, zero failures. |
| Windows `npm run check` | Source, types, coverage, package and isolated CLI installation pass. Coverage: 96.25% lines, 88.97% branches, 94.34% functions. |
| Native Linux `npm ci --ignore-scripts`, typecheck, `npm test` | Pass; 1,099 tests, zero failures or skips after a clean build. |
| Offline Python laboratory | 38 tests pass, including fragmented escapes/UTF-8 and reaping a child after parser failure. |
| Four actual probe references | Session, search, redaction and transcript correctness pass on Windows and Linux. |
| Six broken probe implementations | Empty catalogue, missing load, discarded checkpoint, always-missing search, identity redaction, empty viewport: all rejected by correctness assertions. |
| Public evaluator calibration | Config-edit reference 53/53 and file-server reference 57/57; all eight deliberate evaluator mutations detected. |
| Integrated context tests | Interruption at 40 columns and disconnection at 120 columns preserve compaction and resume without replay. |

An initial extra Linux invocation used `node --test` without preparing the
checkout: 15 failures reported missing `node` in shell PATH or missing compiled
runtime. The subsequent run supplied native Node in PATH, installed the locked
development dependencies and used `npm test` to clean-build first. It passed
without a source change. The incomplete invocation is not release validation.
Remote CI and the complete six-job OS/Node matrix have not been run for this
uncommitted working tree; the added offline job is exercised locally in WSL.

## PTY parser comparison

`Terminal.feed` alone, fresh 140x40 screen, ASCII input, no PTY/provider/disk I/O
inside the interval. One warm-up per implementation and size; six ABBA blocks,
twelve measurements per implementation and size. Whole-screen state, cursor and
pending text agree; the automated tests cover fragmented controls and UTF-8.

| Characters | Base median [min, max], ms | Candidate median [min, max], ms |
| ---: | ---: | ---: |
| 4,096 | 0.685 [0.624, 0.783] | 0.509 [0.494, 0.558] |
| 16,384 | 3.231 [3.049, 3.472] | 2.070 [2.002, 2.282] |
| 65,536 | 38.487 [38.139, 38.920] | 8.412 [8.158, 8.743] |

The largest chunk takes approximately 78% less parser time (4.6x throughput).
This does not imply the same improvement in Jecode task duration or invalidate
old comparisons. Actual chunk sizes and parser time are now recorded numerically
in live-run outcomes; their contribution must be observed in a new campaign.

For reproduction, obtain the base `terminal.py` with `git show` at the commit
above, import both modules separately, initialize the same inert screen as
`test_terminal.py`, and time only `feed('x' * size)` with `time.perf_counter()`.
Keep screen construction and equality assertions outside timing. SHA-256 of
the measured parser sources, with LF line endings:

- Base: `c1db5d5a1ead8392da0e13afd44365dd97df58baf2af86012a86b2a6599861c0`.
- Candidate: `7921c68860968b17f9438fdaf93fce3d9bb5df2b6b1d53d446791bffa066c6bd`.

## Production paths observed through the new fixtures

The integrated 60-column case, after tokenizer warm-up in `bench:context`,
processed twelve 8,192-character results plus 32,768 characters of earlier
context. It used production Responses measurement and wire handling on loopback,
reached 29,466 measured input tokens in a 32,000-token fixture window, compacted
once, interrupted a partial stream, exited, resumed and completed a new turn.
All twelve canonical results and their ordered receipts survived; no historical
tool or generation was replayed before new input. Fourteen ordinary generation
requests and one summary were recorded; HTTP fallback was not exercised here.

| Observation | Value |
| --- | ---: |
| Full fixture lifecycle, including checks and TUI scheduling | 374.623 ms |
| Interruption to settled checkpoint | 46.211 ms |
| Durable resume load | 7.677 ms |
| Input measurement median / maximum, 22 observations | 1.280 / 12.677 ms |
| Checkpoint median / maximum, 13 observations | 13.731 / 19.906 ms |
| Request preparation median / maximum, 14 observations | 2 / 13 ms |

These are observations within one fixture execution, not independent live
sessions or inference timings. They establish a repeatable integration workload.
The synthetic transport uses fixed responses and cannot assess summary quality
or model decision-making. Canonical comparisons deliberately exclude transient
provider transport metadata, as required by the existing persisted format.

The regular session probe now includes 50 and 200 sessions. With one node per
session, catalogue medians were 19.437 and 72.024 ms. Twelve sessions measured
5.851 ms at one node and 5.157 ms at 200 nodes. These remain local observations,
with no new timing gates or candidate/base improvement claim.

A separate instrumented catalogue drill-down wrapped `node:fs/promises`
`lstat`, `stat`, `realpath`, `open`, `readFile`, `readdir`, and `opendir`, synchronized
builtin ESM exports, and counted calls only during `DurableSessionStore.list`.
The 200-session list opened 800 metadata files, inspected the directory once,
and made 204 realpath and approximately 8,100 lstat calls. These are JavaScript
API counts, not an operating-system syscall trace. Work scales with candidate
count while checking anchors, heads and leases. Capping discovery at 64 names
would select the wrong sessions; a global cache/index needs a separate design
with invalidation and race evidence before changing this safety boundary.

Cold tokenizer observations were 141.880 ms / 21.626 ms maximum stall on the base
and 140.628 ms / 18.689 ms on the candidate. The previously reported 137–181 ms
stalls did not recur on this host. A separate, instrumented inspector CPU profile
collected 150 samples: 64 attributed to `vocabulary.load`, 19 to GC, 19 idle,
and 20 to native base64/Latin-1 conversion. Sampling does not isolate a single
stall or prove which loader phase requires a change. Keep the owned loader and
its checksum/yield guarantees; retain mixed-input and cold-start observations
before considering a new format or cache.

## Next evidence

Review and merge these laboratory changes after CI. Establish clean collections
using the same new methods. For the next live comparison, freeze held-out complex
tasks and a blind quality rubric before generation, then counterbalance clients
on the same native environment. Evaluate scope, accepted behavior, failure
recovery and time to a correct outcome together. Neither this work nor the small
historical trials establishes general superiority over Codex.
