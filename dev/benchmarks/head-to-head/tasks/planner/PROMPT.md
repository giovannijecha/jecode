Harden this build planner and implement dependency-aware target selection. Finish
the implementation, regression tests and README. Use one agent, Node standard
library only, no dependencies, web, external services, delegated agents or Git
history changes. Work only in this project; do not inspect sibling directories.

Keep `planBuild(document, options = {})` exported from `src/index.js`, and the
CLI `node src/cli.js FILE [--target ID ...]`. The planner describes work; it must
never execute task commands. Preserve the existing module boundaries where useful.

Contract:

1. A document is a plain JSON object with exactly `tasks`, an array. Each task is
   an object with exactly `id`, `deps`, and `duration`. IDs are nonempty ASCII
   letters, digits, underscores or hyphens; case-sensitive and unique. `deps` is
   an array of distinct existing IDs, without self-dependencies. `duration` is a
   nonnegative safe integer. Reject missing/extra fields and wrong types. Empty
   task lists and zero durations are valid. Never mutate the document or options.
2. Validate the entire graph before selecting targets: unknown dependencies,
   duplicate IDs and cycles are errors even in excluded components. Handle deep
   dependency chains without recursive call-stack overflow.
3. Options accepts only `targets`, a nonempty array of distinct existing IDs when
   present. Omission selects all tasks. Selection includes each target and its
   complete transitive dependencies. Reject unknown options, wrong types, empty
   arrays, unknown or duplicate targets.
4. Return exactly `{ order, waves, totalDuration, criticalPathDuration }`.
   `order` is the lexicographically smallest available-task topological order:
   after each emitted task, newly available tasks compete immediately with all
   remaining ready tasks. Use JavaScript string comparison, not locale sorting.
   `waves` groups tasks by dependency depth (roots at depth 0), each wave sorted
   by the same string order. Count each included task once in `totalDuration`;
   `criticalPathDuration` is the maximum sum along an included dependency path.
   Empty graphs return empty arrays and zero durations. Reject unsafe included
   total/path sums; an excluded component's duration is not aggregated.
5. CLI options may appear before or after FILE; --target may repeat with distinct
   values. --help alone succeeds and prints usage. Reject unknown flags, missing
   values, duplicate targets, missing/extra files and --help combined with anything.
   A successful plan prints exactly one JSON value plus newline to stdout.
   Argument/read/JSON/graph errors exit 1, print a concise stderr error without a
   stack trace and leave stdout empty. All reads are read-only.
6. Add deterministic tests for graph validation, ordering, target closure,
   durations, deep graphs and CLI failures. Keep the existing public exports
   working, update README/examples, run all tests and review/fix your changes.

Report the result and the verification actually performed. No clarification needed.
