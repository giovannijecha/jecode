# Build plan

A read-only planner for tasks, dependencies and estimated durations.

Run `npm test` or `npm run plan`. The public `planBuild` function lives in
`src/index.js`; CLI argument handling, loading, graph operations and reporting
are separated. The initial implementation assumes trusted input and ignores
target selection. It never executes a task.
