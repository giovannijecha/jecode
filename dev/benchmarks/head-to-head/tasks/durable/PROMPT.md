Extend this existing read-only build planner with a reliable asynchronous task
executor and durable checkpoints. This is an implementation task: deliver the
working code, meaningful deterministic tests, and updated documentation, then
verify and review the final change. Do not commit or publish anything.

Keep every existing export, planner/CLI behavior and supported input representation.
Keep the planner CLI read-only: do not add execution flags. No dependencies or
network access. Use Node.js standard-library modules and cohesive ESM modules.
Task commands are never shell strings; execution occurs only through an injected
callback. Keep existing tests meaningful rather than rewriting expectations to
hide regressions.

Export these additions from `src/index.js`:

1. `executePlan(document, options)` returns a Promise. `options` is a plain object
   containing only `run`, optional `concurrency`, `signal`, and `checkpoint`.
   `run(task, { signal })` is required; it may return a value or Promise, throw or
   reject. Its return value is ignored. Concurrency defaults to 2 and must be an
   integer from 1 through 32. `signal`, when present, is an AbortSignal. A checkpoint
   adapter has callable `load()` and `save(snapshot)`; they may be asynchronous.
2. `createCheckpointStore(file)` returns such an adapter. `file` is a nonempty
   string. It stores one bounded JSON checkpoint atomically at that location.

Execution contract:

- Validate the complete document using the existing planner contract (including
  cycles, unsafe aggregate durations and excluded components) and validate options
  before reading a checkpoint or invoking any worker. Do not mutate caller inputs.
  Preserve plain and null-prototype objects, including non-enumerable own fields
  already accepted by the planner. Capture the task definition before asynchronous
  work so later caller mutation cannot change the active execution or checkpoint.
- Run ready tasks in JavaScript string order, refilling free slots as tasks settle;
  do not wait for unrelated slow tasks to finish a wave. At most `concurrency`
  tasks may occupy slots. A successful task occupies its slot until its checkpoint
  commit finishes; a dependent cannot start before prerequisites are durably saved.
  Empty graphs complete without calling `run`.
- A worker failure blocks its transitive dependents, but independent work continues.
  Never invoke a task twice in one execution. Support synchronous throws, rejected
  Promises, deep graphs and prototype-like IDs without recursion overflow or lost IDs.
- Return exactly `{ status, completed, failed, blocked, pending }`: all four arrays
  contain sorted IDs, are disjoint and cover the entire graph. Completed includes
  checkpoint-restored successes. Failed contains worker failures; blocked contains
  their transitive dependents; pending contains tasks not started due to cancellation.
  Status is `cancelled` if externally aborted, otherwise `failed` if any worker
  failed, otherwise `completed`.
- An already-aborted signal starts no work. Once aborted, start no more tasks and
  abort the signal supplied to every active worker. Await all started workers and
  pending persistence before resolving. A worker that resolves after abort still
  completed and is saved. A worker rejecting after external abort remains pending
  for retry, rather than becoming a permanent failure. Compute blocked tasks from
  actual worker failures that preceded cancellation.
- Adapter failures reject execution, stop dispatch and signal active workers.
  Await all already-started workers before rejecting; no later background writes
  or worker starts may occur after the returned Promise settles. Preserve the
  original adapter failure. Do not issue further saves after a save fails.

Checkpoint contract:

- `load()` is called once after validation. `null` means no checkpoint. Before any
  worker starts, save an initial empty checkpoint if none exists. Save each newly
  successful task before considering it completed or unblocking dependents. Saves
  never overlap. No checkpoint option means execution without persistence.
- Snapshot shape is exactly `{ version: 1, tasks, completed }`. `tasks` is a
  canonical array of explicit `{ id, deps, duration }` records sorted by ID, with
  each deps array sorted; completed is a sorted array of unique IDs. Snapshot
  values are detached from inputs, worker tasks and previous snapshots.
- Reject malformed/unknown-version checkpoints, unknown or duplicate completed
  IDs, successes missing any dependency, or a task-definition mismatch before
  dispatch. Reordering tasks/dependencies alone is equivalent; changing an ID,
  edge or duration is not. Do not silently repair or discard invalid state.
- Resuming skips saved successes and retries tasks that did not succeed. Worker
  failure details are deliberately not persisted. Never claim exactly-once external
  side effects: a process can stop after a worker effect and before its save.
  Document that window and require idempotent callbacks for safe crash retry.

File adapter contract:

- A missing file loads as null. Malformed JSON, an invalid snapshot, non-regular
  files and files larger than 1 MiB are errors. Do not follow a target symlink.
- Save captures and validates a detached snapshot before its first asynchronous
  boundary. Invalid saves leave an existing file untouched. Queue concurrent calls
  on the same adapter in invocation order, without a rejected save poisoning later
  valid calls. One process owns the file; cross-process locking is out of scope.
- Write UTF-8 JSON plus newline via a unique temporary file in the same existing
  parent directory and atomic rename; clean up temporary files on failure. Check
  target type again before replacement. Preserve the last valid checkpoint if a
  write fails. Do not create missing parent directories. Limit encoded saves to
  1 MiB as well. Loading and saving must not expose mutable internal state.

Add tests that exercise real asynchronous overlap, refill order, dependency commit
barriers, failure propagation, cancellation while workers are active, delayed and
failed saves, restart from persisted state, invalid state, and real file storage.
Use deterministic synchronization for races. Review the complete change and run
the original suite plus your new tests. Update README with examples, API behavior,
recovery limits and the distinction between read-only planning and explicit execution.
