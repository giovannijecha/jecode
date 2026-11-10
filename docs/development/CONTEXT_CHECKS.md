# Checking long contexts without replaying model calls

Use offline checks to screen context changes before paying for another model
session. They exercise context serialization and projection with the owned
Rust code. They instantiate no backend, contact no provider and read no account
configuration.

A replay checks a fixed history. It cannot measure how a model would change its
next decision, tool calls or output in response to a different context. Keep
quality and weighted-token comparisons as live measurements.

## Replay saved sessions

Select explicit journal files from an existing session. On PowerShell:

```powershell
$env:JECODE_REPLAY_JOURNALS = 'C:\path\to\session.jsonl'
cargo test --release --locked --offline sessions::journal::replay_tests::offline_saved_context_replay -- --exact --ignored --nocapture --test-threads=1
Remove-Item Env:JECODE_REPLAY_JOURNALS
```

Multiple files use the platform path separator: `;` on Windows and `:` on Unix.
Saved working-directory paths must be valid absolute paths for the replay host.
Use the original platform for the original journal.

The probe reads each current-format journal checkpoint in order, merges its state patch using
the native codec, restores returned-message state from events, and rebuilds the
context projection. At every checkpoint it checks context bounds and identical
projection across `Context::value` / `Context::parse`, with the same
returned-message mask on both sides. Invalid continuation offsets, unsupported
formats, malformed JSON or context fields, and an incomplete final checkpoint
fail the probe. Source journals are opened for reading and remain unchanged.

This is a context-projection check. It does not independently validate session
identity, the complete message/tool/event sequence or returned-event persistence.
Those belong to the production session reader and its existing tests. A passing
replay is not a general claim that Jecode could open an arbitrary edited journal.

The output reports checkpoints, messages and elapsed local
processing time. `peak_projection_bytes` is the sum of each projected message's
JSON bytes, excluding array separators, regenerated environment instructions and
provider-specific encoding. `provider_requests` is zero. These byte counts are
not provider tokens or a cache-hit measurement. The probe reports no semantic
quality score.

The saved-session probe is ignored by the normal suite. A small ordinary test checks
the replay reader against native state patches, a broken continuation and an
uncommitted tail, without any private journal fixture.

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
[benchmark report](AGENT_BENCHMARK.md#forty-turn-confirmation). All 6,499 saved
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

The experimental build reads session state version 2 in journal version 4.
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
this review. Normal-budget quality and the token-saving target remain live
measurement questions.

## Spend live calls where they decide something

1. Run the offline replay to catch serialization and projection defects.
2. For a candidate with a concrete hypothesis, use short live comparisons at
   normal and tight budgets, with fresh rival runs in the same time block. Keep
   quality, recall, tool compliance and all request usage in the comparison.
3. Test distinct task families before confirming the selected runtime on the
   longer session. Fixed-response replay cannot establish a quality-preserving
   token saving: the model may take a different path in a live run.
4. Reserve full-duration runs for questions that require them. A test-only or
   documentation change does not justify rerunning an unchanged model session.

For a known late-session failure, screen the next decision in isolation before
repeating the full trajectory. The maintainer's fixture now supports this method:

1. Select the last complete native checkpoint before the target request, with
   no pending tools or partially returned results. Preserve the original journal.
2. Restore the corresponding project snapshot in a fresh directory and allocate
   a new native session identity and temporary area. Verify the source path set
   and hashes; relocate only session-owned paths. Keep scorer metadata outside
   the model-visible project.
3. Validate the seed with offline replay, then resume it with local commands
   only. Confirm that no provider request was made and the source stayed intact.
4. Freeze the candidate binary, model, effort and settings. Send the original
   next request once, then run the unchanged hidden and project suites. Compare
   earlier failures with the precondition so inherited failures stay visible.

This sends one live task turn per prepared case instead of regenerating its
earlier turns. It measures that decision in the prepared context. Source hashes
do not independently establish historical documentation or scratch contents;
relocated state also changes the wire prefix and cache behavior. Keep these
quality screens separate from full-session quality and weighted-token results.
The fixture preparation is maintainer tooling, not a new Jecode CLI command.
Never copy plan OAuth records to seed an isolated home; use an independently
authenticated home or an approved API-key backend.

Elapsed-time properties remain separate: cache expiry, account refresh, idle
connections and recovery after a real pause require real elapsed time and their
actual provider or operating-system boundary. Waiting itself spends no model
tokens. A synthetic clock or a compressed pause does not verify provider cache
expiry. A real ten-hour quality claim still needs corresponding live evidence.
