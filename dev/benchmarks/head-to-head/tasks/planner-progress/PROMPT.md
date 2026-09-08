Extend the working build planner to account for tasks that have already completed.
Keep the existing APIs, CLI behavior and module boundaries where useful. Use one
agent, Node standard library only, no dependencies, network, delegated agents or
Git history changes. Work only in this project, not sibling directories.

Start by reading the existing implementation and tests. The implementation is
already functional: preserve its behavior when no completed tasks are supplied.

Add optional `completed` to `planBuild(document, options = {})`:

1. When present, `completed` is an array of distinct existing task IDs. An empty
   array is valid. Reject other types, duplicate/unknown/non-string IDs, including
   an explicitly undefined value. Existing `targets` validation stays unchanged.
2. Validate the whole document and graph before applying targets or completed
   tasks. Cycles, unknown dependencies and malformed tasks remain errors even
   when all offending tasks are completed or outside the selected work.
3. Starting from the explicit targets, or all task IDs if targets are omitted,
   traverse dependencies until a completed task is reached. Completed tasks are
   omitted and cut traversal: their prerequisites are not needed through that
   path. A prerequisite is still included if it is independently targeted or
   needed through another unfinished path. Completion does not imply completion
   of any other ID. Completed IDs outside selection have no effect.
4. Plan only the remaining work. Dependencies on completed tasks are satisfied.
   Ordering, waves, totals and critical path follow the existing rules over the
   remaining graph. Completed durations are not added. Reject unsafe remaining
   sums. No remaining work returns empty order/waves and zero durations.
5. Never mutate document, tasks, dependency arrays or options, including frozen
   inputs. The planner remains read-only and never executes task commands.
6. Add repeatable `--completed ID` to the CLI, combinable with `--target ID` and
   FILE in any order. Reject duplicate IDs, missing values and invalid inputs.
   Keep --help-alone and the existing error/output contract. Errors must remain
   concise and safe to display: at most 1024 characters including the newline,
   no raw terminal control characters other than the final newline. Ordinary
   error wording is not prescribed. Reserved --help/--target/--completed tokens
   need not be accepted as CLI ID values; the API must accept them as valid IDs.

Example: a (2), b depends on a (3), c depends on a (5), d depends on b and c (7).
With target d and completed b, remaining order is a,c,d, total is 14 and critical
path is 14. With target b and completed b, the plan is empty. With targets a,b and
completed b, only a remains. Without targets and completed b, a,c,d remain.

Add meaningful deterministic regression tests, preserve existing tests, update
README/help/examples as appropriate, run the relevant checks and review the final
changes. Report checks actually performed and any remaining limitation. No
clarification is needed.
