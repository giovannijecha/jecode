# Checking long contexts without replaying model calls

The replay command below applies to current-format journals (version 4, session
state 2). The latest recorded screens, with their measured formats and limits,
are in [Context screening, November 2026](../reports/CONTEXT_SCREENING_2026-11.md).

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
