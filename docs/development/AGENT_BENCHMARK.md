# Jecode vs Codex and Claude Code

For the current context work, see the [November 8 results](#selected-native-build-2026-11-08)
and [forty-turn quality scores](#forty-turn-confirmation). The requested general
25% reduction at equal quality remains unconfirmed. The five-turn results below
are historical.

Jecode used fewer cost-weighted tokens than both first-party harnesses on the
maintainer's five-turn coding benchmark. With Claude it also finished sooner;
with GPT, Codex finished sooner. Every run passed all 21 hidden acceptance tests
and its project test suite.

These measurements were recorded on 2026-11-04 on Windows. Jecode used account
sign-in. They describe development builds before the transport, Unix and TUI
changes prepared for 0.3.5, not a new benchmark of the 0.3.5 release.

## Results

Arithmetic means, rounded. Each comparison matches the primary model and high
effort; descendant model choices are noted where verified.
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
- Other Jecode development variants ran the same evening and are not reported:
  they tested ideas that were later changed or removed. The reported builds carry
  the design that was kept, chosen with this same fixture, so the gap could be
  smaller on other projects. Every rival run from that evening is reported;
  earlier rival runs are not, so each comparison shares the same evening.
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

| CSV column | Meaning |
| --- | --- |
| `model`, `harness` | The model and the agent with its version. |
| `configuration` | The setup. For Jecode with GPT, `build-a` is the development binary and `final` the final candidate. |
| `sample` | The maintainer's run number within that harness and model. Gaps are normal. |
| `weighted_tokens` | Cost-weighted tokens, as defined above. |
| `model_requests` | Requests counted in the totals, including subagents and collaborators. |
| `active_seconds` | Active time, as defined above. |
| `hidden_passed`, `hidden_total` | Hidden acceptance tests passed and run. |

These results support lower weighted token use on this workload. They do not
establish universal superiority, a plan-usage multiplier, API-key performance,
or results on Linux and macOS. The later native HTTP and WebSocket work has
separate [before/after measurements](PERFORMANCE.md#provider-transports);
those results must not be added to this comparison to predict a new lead.

## Exact records, development branch, 2026-11-07

A later evening measured the exact native records described in
[Architecture](ARCHITECTURE.md) against Claude Code 2.1.319 and Codex 0.167, on
the same Windows machine, primary models, effort and weights as above. Each Jecode row is
one build of the `context-engine` branch. Unless noted, the rival ran in the same
hour. Every six-turn run passed all 21 hidden tests; the forty-turn scores are
reported separately below. These figures are not in the CSV.

| Workload | Model | Jecode, mean | Rival, mean | Tokens | Time |
| --- | --- | ---: | ---: | ---: | ---: |
| Six turns, no pauses, 3 runs each | Sonnet 5.5 | 566–682k · 292 s | Claude Code 1,013k · 379 s | −44%..−33% | −23% |
| Six turns, no pauses, 4 runs each | GPT-6.1 Sol | 431k · 456 s | Codex 522k · 489 s | −17% | −7% |
| Six turns, budget near 60–70k, 3 runs each | Sonnet 5.5 | 634–804k · 299 s | Claude Code 1,090k · 635 s¹ | −42%..−26% | −53% |
| Six turns, budget near 60k, 3 runs each | GPT-6.1 Sol | 557k · 476 s | Codex 528k · 641 s | +5% | −26% |
| Six turns, budget near 100k, 3 runs each | GPT-6.1 Sol | 443k · 486 s | Codex 702k · 512 s | −37% | −5% |
| Forty turns, 2 runs each² | GPT-6.1 Sol | 4,716k · 78 min | Codex 9,557k · 113 min | −51% | −31% |

The Sonnet token intervals reflect unknown cache-write lifetimes, not run
variation or a confidence interval. Those Jecode journals recorded aggregate
writes while requests mixed a one-hour shared prefix with five-minute blocks.
The lower bound prices every unknown write at 1.25× and the upper bound at 2×.
Claude Code's transcripts reported both buckets. The normal Sonnet pair began in
the same minute.

¹ Claude Code ran about six hours earlier the same day; at a 10% threshold its context peaked at
73k. ² An earlier build of the branch, before the fixes found in review. Its hidden
tests passed 168–169 of 171 against Codex's 169–171; its recall probes scored
56–57 of 57 against 54–55.

Codex's runs vary widely with its subagents (405–716k in the first GPT row and
378–912k in the last), so the GPT margins are the least certain. On the paused
five-turn benchmark above, the branch build's five-minute configuration spans
624–771k weighted tokens from the same uncertainty; the one-hour configuration
averaged 696k, inferred from its known setting (3 runs each, same evening). These
historical counters do not establish a cost advantage for either lifetime.

### Selected native build, 2026-11-08

The retained build includes the record and failure fixes plus provider-reported
cache-write lifetimes. Two fresh GPT comparisons used four runs per harness,
with Codex starting alongside Jecode in each block. All runs passed the 21 hidden
tests. Jecode scored 6/6 recall points in every run; the no-tools probes made zero
tool calls in both harnesses.

| Workload | Jecode weighted tokens, mean (run range) | Codex weighted tokens, mean (run range) | Tokens | Active time, Jecode / Codex |
| --- | ---: | ---: | ---: | ---: |
| Six turns, normal budget | 420k (400–448k) | 627k (437–887k) | −33% | 431 / 462 s |
| Six turns, budget near 60k | 553k (458–694k) | 632k (427–724k) | −12.5% | 466 / 731 s |

Both used GPT-6.1 Sol at high effort for the primary conversation on Windows,
fresh copies of the same fixture and no deliberate pauses. Codex additionally
ran three GPT-6 Sol/high descendant rollouts in the normal block and four in the
tight block. Their usage is included at the same fixed GPT-family weights;
the model match is on the primary conversation. Weighted tokens include all six
turns; active time includes only the five work turns. The fixed weights above
include descendant usage and exclude session titles and separate helper models. Several
sessions shared the machine and network. Lever selection used this same fixture.

The normal-budget mean exceeds the 25% target in this block, while the tight
budget misses it. Codex's broad run ranges remain material. These results do not
establish a 25% minimum on arbitrary tasks or across all backends.

### Tight working-tail follow-up, 2026-11-08

A later controlled block tested retaining more complete recent exchanges during
work at a tight budget. The candidate changed only the working-tail allowance
from `window / 4` to `min(max(window / 4, 30,000), window / 2)`. Digest triggers,
gain thresholds and new-turn behavior stayed fixed. At windows of 120k or more,
the allowance was unchanged. The hypothesis was fewer subsequent rereads; it
still needed to offset sending a larger live tail.

Four frozen control runs, four candidate runs and four fresh Codex runs used the
same six-turn fixture and a budget near 60k on Windows. Primary conversations
used GPT-6.1 Sol/high. The block
ran from 09:22 to 09:48 UTC in two matched waves, each starting two runs per
group together. Several sessions shared the machine and network; local verification
also ran during the block. Codex ran seven GPT-6 Sol/high descendants; their
usage is included with the fixed GPT weights. The model match is on the primary
conversation. Token totals cover T1-T6; active time covers T1-T5.

| Build, 4 runs each | Weighted tokens, mean (run range) | Against Codex | Active seconds, mean | Recall / 6 |
| --- | ---: | ---: | ---: | ---: |
| Retained control | 571,590 (520,838-625,107) | -20.0% | 516.0 | 5-6 |
| Larger working tail | 604,266 (462,964-671,828) | -15.4% | 513.5 | 6 |
| Codex | 714,629 (586,261-889,621) | Reference | 717.3 | 2-5 |

All 12 runs completed all six turns and passed 21/21 hidden checks. All 12
no-tools probes made zero calls and were verifiable. The original recall rubric
and scores are retained, including the control's one uncertain function-name
answer. No generated workspace or acceptance criterion was changed.

The candidate spent **5.72% more weighted tokens than the control**. Mean fresh
input rose from 306,258 to 345,940; output stayed near 20,240 tokens, and request
count fell from 40.5 to 39.0. These small samples establish no general causal
claim about rereads, but provide no reason to adopt the candidate. It was
rejected and the retained context policy is unchanged. No additional Claude or
forty-turn session was needed to select a rejected candidate.

The retained control still missed 25% in this fresh block: the required maximum
was 535,972, leaving 35,618 weighted tokens per run. This result supplements the
earlier -12.5% tight block; neither block establishes the requested general floor.

### Forty-turn confirmation

The same retained executable completed two 40-turn sessions per primary model.
The following cost/time comparisons use older rival runs from November 7, not
same-hour controls. Claude Code stopped at 28 and 30 turns, so its comparison
ends at the common turn 25. Token counters are cut at that turn's end, before any
following turn; pauses are excluded from active time.

| Comparison, 2 runs each | Jecode weighted tokens, mean (run range) | Rival weighted tokens, mean (run range) | Tokens | Active time, Jecode / rival |
| --- | ---: | ---: | ---: | ---: |
| GPT-6.1 Sol, 40 turns | 4.637M (4.469–4.805M) | Codex 9.556M (7.401–11.712M) | −51.5% | 74.9 / 113.0 min |
| Sonnet 5.5, through turn 25 | 3.024M (2.994–3.053M) | Claude Code 12.330M (11.065–13.596M) | −75.5% | 24.7 / 56.8 min |

The complete Sonnet sessions averaged 5.820M weighted tokens and 47.3 minutes
over all 40 turns. Both reported the cache-write lifetime buckets, with no
unknown writes. These totals use their actual five-minute and one-hour counts.

Quality was scored independently after completion, on each final workspace:

| Run | Hidden assertions | Project suite, passed / total | Recall points | Standing rules |
| --- | ---: | ---: | ---: | ---: |
| GPT 1 | 168/171 | 486/486 | 57/57 | 5/6 |
| GPT 2 | 168/171 | 538/538 | 56/57 | 5/6 |
| Sonnet 1 | 171/171 | 348/349 | 56/57 | 6/6 |
| Sonnet 2 | 170/171 | 355/357 | 55/57 | 5/6 |

The project suites had zero hard failures; Sonnet's remaining one and two cases
were marked TODO. All 20 no-tools probes made zero tool calls, with no
unverifiable probes. Recall and standing-rule scores use the fixture's automated
rubrics, separately from acceptance tests and tool compliance.

Each GPT run failed two period-lock assertions: it refused a reversal even when
given a later open date. One documentation assertion also failed because a
removal note still mentioned the obsolete flag. The older Codex runs scored
169/171 and 171/171. These results therefore do not establish equal acceptance
quality for GPT. The same lock defect occurred in one Codex run; the measurements
do not isolate a compaction cause.

Sonnet 2's single hidden miss is a lexical checker limitation: the test name
describes a root aggregation symptom, while the checker requires one of three
other words. The original automated score is retained above. No generated
workspace was repaired manually, and no acceptance criterion was changed after
observing these results.

The long sessions show lower weighted-token use on this fixture, with the
quality limits above. Together with the tight-budget GPT result, they leave the
requested general 25% reduction at equal quality **unconfirmed**.

### Diagnosing the GPT gap without new model calls

The same 16 short runs above were decomposed from their original usage records,
eight per budget condition. With the fixed GPT weights, tight-budget Jecode's
553,090 mean comprises 305,650 fresh input, 94,013 weighted cache reads and
153,428 weighted output, against Codex's 239,329, 204,390 and 188,556. Neither
reported cache writes in these runs. Reaching 25% below Codex would require at
most 474,207: another 78,884 weighted tokens per Jecode run.

By recorded turn windows, refactoring is the only tight turn where Jecode costs
more: 313,008 against 264,050. Matching that turn alone would still leave about
29,925 to remove. Turn assignment uses Jecode request starts and Codex usage-event
timestamps; costs around a boundary are therefore less directly comparable than
the reconciled full-run totals. Provider usage cannot reliably attribute these
tokens to individual tool-result bytes, rereads or prompt sections, so this
decomposition does not establish a particular optimization's cause or benefit.

The saved GPT checkpoints before the period-lock and obsolete-flag edits retain
the current user requests in the live window, without summary or preview. At
the later edit the earlier lock request also fits in its native record in full.
The saved actions include tests asserting the overly strict lock and documentation
explicitly naming the removed flag. This supports no finding of lost requirements
from compaction. Reconstructed projection does not prove the complete historical
wire request or how the provider attended to it; those bodies were not saved.

For cheaper screening, [offline context checks](CONTEXT_CHECKS.md) exercise saved
context checkpoints and sustained record growth without provider calls. Their results
remain separate from live quality, cache and weighted-token comparisons.
