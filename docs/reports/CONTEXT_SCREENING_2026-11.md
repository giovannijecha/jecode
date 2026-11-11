# Context screening, November 2026

| Record | Scope |
| --- | --- |
| Dates | 2026-11-08 and 2026-11-09. |
| Measured builds | Format-3 replay reader; frozen reference binary before stabilization fixes; reviewed journal-version-4, session-state-2 build. Exact binary revisions were not recorded. |
| Platforms | Replay screening: Windows with Rust 1.95.0. Live screen: host and toolchain were not recorded. The review also ran on Ubuntu under WSL, as detailed below. |

The format-3 and model-free-record results are historical. They do not measure
0.5.5's append-only requests and model compaction, or establish a token or speed
advantage. The current replay procedure is in
[Context checks](../development/CONTEXT_CHECKS.md).

## Executed screening, 2026-11-08

On the maintainer's Windows host with Rust 1.95.0 and the release test profile:

| Probe | Context checkpoints | Source messages | Processing time |
| --- | ---: | ---: | ---: |
| 360 synthetic turns | 360 | 1,442 | 3.85 s |
| Saved GPT session 1 | 1,838 | 523 | 29.86 s |
| Saved GPT session 2 | 1,870 | 559 | 34.42 s |
| Saved Sonnet session 1 | 1,445 | 790 | 7.98 s |
| Saved Sonnet session 2 | 1,346 | 731 | 7.36 s |

The four saved sessions are the completed 40-turn runs in the
[benchmark report](AGENT_BENCHMARK_2026-11.md#forty-turn-confirmation). All 6,499 saved
checkpoints passed in 79.61 seconds of replay processing. The first release-test
compilation took 6m48s separately. Source journal hashes were identical before
and after, and no benchmark provider call was made. These are local screening
times, not agent-task speedups or a ten-hour quality result. These measurements
used the then-current format-3 reader. The current reader accepts only journal
version 4 and session state version 2, so it cannot replay those archived
sessions unchanged. The synthetic turns exercised model-free records, which
were later retired with their probe.

## Live behavioral screen, 2026-11-09

Two API-backend sessions on the maintainer's fixture, with unchanged hidden
checks, standing rules and a memory-only recall probe. Each ran in a new API-key
home at `auto_compact_percent` 5, with `high` effort and a 5-minute Claude cache.
The frozen binary was the reference built before the stabilization fixes, not
the final reviewed source. A driver outside Jecode capped attempts and time.

| Backend · model | Turns | Compactions | Observed attempts | Cached input | Hidden checks |
| --- | --- | ---: | ---: | ---: | --- |
| `openai-api` · `gpt-6.1-sol` | 6 of 6 | 6 | 44 | 76.2% | all passed |
| `anthropic-api` · `claude-sonnet-5-5` | 1–3, T4 cut at the cap | 5 | 44 | 85.5% | T4 README check failed |

Every recorded compaction used the in-place summary path and completed on its
first attempt. All recorded attempts reported usage and none reported a failure.
Forced termination can leave an in-flight request unrecorded, so the Anthropic
totals are known usage, not a proof of all billed work. Summaries took 48% (GPT)
and 38% (Sonnet) of recorded output tokens. OpenAI completed all six turns and
scored 5/6 on automatic recall, with zero tool calls. Passing the hidden checks
does not independently prove the quality of every code or documentation change.

Anthropic never received T5. A later manual resume sent the T6 recall prompt and
noticed the unfinished rename, using no tools. This separate request adds 43,569
input and 848 output tokens; it is excluded from the original run's attempts,
usage and elapsed time. It read 14,268 cached input tokens after a 12-minute gap,
consistent with expiry of the requested five-minute cache. This observation
does not establish cache expiry as the only cause of prefix reuse.

Both sessions reread files after compaction, but their contribution to total
cost was not isolated. The corrected metrics separate the later manual probe;
the original mixed-scope output remains archived. This is a behavioral screen
at an artificial budget, not a measured saving or a comparison with another
harness. A complete Anthropic run and a screen at the default 80% threshold
remain unexecuted.

## Review and current-format checks, 2026-11-09

The reviewed build reads session state version 2 in journal version 4.
Older session formats are rejected without rewriting their files. Digest
migration, persisted preview state and the conversation-level legacy origin
adapter were removed. Backend/model switching still uses per-message origins.
Tool-result previews remain local to the portion-summary transcript; they no
longer become persistent context state. The configuration uses `selection` and
`backends`; a retired root-level OpenRouter block is rejected without rewriting
the configuration.

After updating fixtures to the current writer's output, executed checks passed:

| Host | Build, fmt, clippy | Full suite | Explicit native checks |
| --- | --- | ---: | --- |
| Windows, Rust 1.95.0 gnullvm | passed | 2,371 | startup, fullscreen lifecycle, active-history Esc, direct PTY input |
| Ubuntu on WSL, Rust 1.95.0 | passed | 2,405 | terminal lifecycle, startup |

The Windows active-history check uses ConPTY and a local gated stream: it
exercises typing, tool output, resize, Esc and terminal restoration. These
checks do not measure live-provider latency. macOS was not executed. The
previously observed Linux process-stop flake did not recur and is not claimed
fixed.

Offline driver stubs also verified first-turn attempt limits, failure of the
attempt counter, isolated-home rejection and resume identity checks. Limits
count completed journaled requests, including summaries and titles; polling
does not bound unreported in-flight work. No new provider call was made during
this review. Normal-budget quality and token use on the current context path
remain live measurement questions.
