# Checking long contexts without replaying model calls

Use offline checks to screen context changes before paying for another model
session. They exercise the native record renderer, context serialization and
projection with the owned Rust code. They instantiate no backend, contact no
provider and read no account configuration.

A replay checks a fixed history. It cannot measure how a model would change its
next decision, tool calls or output in response to a different context. Keep
quality and weighted-token comparisons as live measurements.

## Sustained record growth

After the [build prerequisites](TESTING.md), run:

```text
cargo test --release --locked --offline sessions::journal::replay_tests::offline_record_endurance -- --exact --ignored --nocapture --test-threads=1
```

The fixture appends 360 synthetic turns with a 64 KiB read result per turn. It
records each completed turn on a fixed schedule and uses a 32k estimated-token
budget. It exercises both record rollup levels, validates context boundaries,
checks projection equivalence across context serialization at every checkpoint,
keeps the next request live and checks that every original payload remains in history.

This tests growing history and repeated reductions. The fixed schedule does not
exercise the agent's adaptive reduction decisions, model summaries, actual tool
execution or a real ten-hour duration. It does not write a session journal.
Existing agent tests cover the reduction
decisions and summary fallback separately.

## Replay saved checkpoints

Select explicit journal files from an existing session. On PowerShell:

```powershell
$env:JECODE_REPLAY_JOURNALS = 'C:\path\to\session.jsonl'
cargo test --release --locked --offline sessions::journal::replay_tests::offline_saved_context_replay -- --exact --ignored --nocapture --test-threads=1
Remove-Item Env:JECODE_REPLAY_JOURNALS
```

Multiple files use the platform path separator: `;` on Windows and `:` on Unix.
Saved working-directory paths must be valid absolute paths for the replay host.
Use the original platform for the original journal.

The probe reads each format-3 checkpoint in order, merges its state patch using
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

The output reports checkpoints, messages, record levels and elapsed local
processing time. `peak_projection_bytes` is the sum of each projected message's
JSON bytes, excluding array separators, regenerated environment instructions and
provider-specific encoding. `provider_requests` is zero. These byte counts are
not provider tokens or a cache-hit measurement. The probe reports no semantic
quality score.

Both large probes are ignored by the normal suite. A small ordinary test checks
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
times, not agent-task speedups or a ten-hour quality result.

## Spend live calls where they decide something

1. Run the offline probes to catch serialization, projection and growth defects.
2. For a candidate with a concrete hypothesis, use short live comparisons at
   normal and tight budgets, with fresh rival runs in the same time block. Keep
   quality, recall, tool compliance and all request usage in the comparison.
3. Test distinct task families before confirming the selected runtime on the
   longer session. Fixed-response replay cannot establish a quality-preserving
   token saving: the model may take a different path in a live run.
4. Reserve full-duration runs for questions that require them. A test-only or
   documentation change does not justify rerunning an unchanged model session.

Late-context probes can evaluate a particular prepared context with fewer live
calls than rebuilding its entire history. They still need a controlled fixture,
an authentic request and independent quality checks, and establish only that
local condition. They are not an implemented shortcut to a ten-hour trajectory.
Never copy plan OAuth records to seed an isolated home; use an independently
authenticated home or an approved API-key backend.

Elapsed-time properties remain separate: cache expiry, account refresh, idle
connections and recovery after a real pause require real elapsed time and their
actual provider or operating-system boundary. Waiting itself spends no model
tokens. A synthetic clock or a compressed pause does not verify provider cache
expiry. A real ten-hour quality claim still needs corresponding live evidence.
