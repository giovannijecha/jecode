# Jecode vs Codex and Claude Code

Jecode used fewer cost-weighted tokens than both first-party harnesses on the
maintainer's five-turn coding benchmark. With Claude it also finished sooner;
with GPT, Codex finished sooner. Every run passed all 21 hidden acceptance tests
and its project test suite.

These measurements were recorded on 2026-11-04 on Windows. Jecode used account
sign-in. They describe development builds before the transport, Unix and TUI
changes prepared for 0.3.5, not a new benchmark of the 0.3.5 release.

## Results

Arithmetic means, rounded. Each comparison uses the same model and high effort.
Token ranges show the lowest and highest run; `k` means 1,000 weighted tokens.

| Model | Harness | Runs | Weighted tokens, mean (range) | Model requests, mean | Active time, mean |
| --- | --- | ---: | ---: | ---: | ---: |
| GPT-6.1 Sol | Jecode, steps and delegation guidance | 7 | 494k (459–541k) | 31.1 | 541 s |
| GPT-6.1 Sol | Codex 0.166 | 6 | 623k (402–764k) | 47.3 | 488 s |
| Claude Sonnet 5.5 | Jecode, 1-hour prompt cache | 4 | 710k (619–794k) | 30.5 | 367 s |
| Claude Sonnet 5.5 | Claude Code 2.1.318 | 3 | 1,107k (984–1,194k) | 46.7 | 522 s |

Relative to Codex, Jecode used **21% fewer weighted tokens** and took **11%
longer**. Relative to Claude Code, it used **36% fewer weighted tokens** and
took **30% less active time**. The [per-run CSV](AGENT_BENCHMARK.csv) contains
the scalar measurements behind these means.

## Workload and controls

- One original Node project of about 2,000 lines; five turns in a saved
  conversation: exploration, a bug fix, a feature, a rename refactor and a
  wrap-up. Each run began from a fresh copy of the same fixture.
- Two 390-second pauses, before turns three and five, to exercise cache reuse
  beyond five minutes. The same prompts reached each harness over standard input.
- High effort, full local permissions and the same model within each pair.
  Jecode used its ChatGPT and Claude account routes. These were not API-key
  comparisons. Provider access and the Claude compatibility caveat are described
  in [Providers](../usage/PROVIDERS.md). The runner did not record the rival
  CLIs' credential source.
- Codex used `--ignore-user-config`; its loaded maintainer instructions and
  skills still contributed an estimated 5–8% of its tokens. Claude Code used
  `--safe-mode`. These configurations do not represent every user's defaults.
- The seven Jecode GPT runs combine four runs of one development binary and
  three of a final candidate, both with steps and delegation guidance. The four
  Claude runs use the final candidate. They are not seven repeats of one
  identical release binary.
- Rivals ran the same evening as Jecode. The GPT runs span several blocks;
  the Claude pair began together. Multiple sessions shared the machine and
  network. This was not an idle-machine latency test or a randomized trial.

After the five turns, hidden tests checked the bug fix, feature and refactor:
21/21 passed for every run. Each agent also passed the project's own test suite.
This checks the fixture's acceptance criteria, not general code quality or
reliability on other projects.

## What the metrics mean

**Active time** is the sum of the five CLI turn durations, including process
startup and tools. It excludes the two deliberate pauses and the later hidden
test run. It is not the elapsed duration of the entire experiment.

**Cost-weighted tokens** use fixed weights within each model family:

| Usage component | GPT weight | Claude weight |
| --- | ---: | ---: |
| Uncached input, excluding cache writes | 1 | 1 |
| Cache read | 0.1 | 0.1 |
| Five-minute cache write | Not separately reported | 1.25 |
| One-hour cache write | Not separately reported | 2 |
| Output | 8 | 5 |

Multiply each component by its weight, then add. For GPT, all input outside
cache reads counts at 1×, with no separate cache-write component. These are the
benchmark's recorded weights, not a current price list. They make repeated cached context
and generated output visible in one cost proxy. Do not compare weighted totals
between GPT and Claude, read them as dollars, or treat them as a measurement of
subscription quota. API bills and plan limits depend on the provider.

Usage and request counts include Codex's descendant subagent rollouts, which
its main `exec` usage alone omits. Jecode's collaborator conversations are also
included. Auxiliary work, such as session titles and separate helper-model
requests, is excluded from the token and request totals. The active-time metric
still includes work performed during each CLI turn.

## Scope and reproducibility

This is a maintainer benchmark, not an independent evaluation. It uses one
fixture, a small sample, particular harness versions and one Windows machine.
The Claude Code group has only three runs. Provider caching and scheduling
vary; the large token ranges are part of the result. Runs with cache misses
remain in the reported totals.

The CSV publishes sanitized per-run counts and durations so the means and
percentages can be checked. The fixture, driver and private transcripts remain
local, so the full experiment is **not independently reproducible from this
repository**.

These results support lower weighted token use on this workload. They do not
establish universal superiority, a plan-usage multiplier, API-key performance,
or results on Linux and macOS. The later native HTTP and WebSocket work has
separate [before/after measurements](PERFORMANCE.md#provider-transports);
those results must not be added to this comparison to predict a new lead.
