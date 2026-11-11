# Reports

These reports preserve dated evidence. Their results apply to the measured
builds, workloads and platforms, not automatically to the current release.
Current procedures belong in [Development](../README.md#development).

1. Record the measurement date, measured version or commit, platform and method.
   When an older source did not record a revision or host detail, say so;
   do not infer the measured binary from the commit that added its documentation.
2. Keep published measurements and conclusions unchanged. Add a new dated
   report for new evidence or corrections. Link maintenance may update paths
   without changing the evidence.
3. Link from development procedures to the latest relevant report, identifying
   its date and scope. Keep measured numbers in the report rather than copying
   them back into the procedure.

| Report | Recorded dates | Scope |
| --- | --- | --- |
| [Agent benchmarks](AGENT_BENCHMARK_2026-11.md) | 2026-11-04, 07 and 08 | Windows comparisons with earlier development builds; [per-run CSV](AGENT_BENCHMARK_2026-11.csv) for the November 4 runs. |
| [Context screening](CONTEXT_SCREENING_2026-11.md) | 2026-11-08 and 09 | Format-3 replay, live behavioral screens and current-format review checks. |
| [Performance results](PERFORMANCE_RESULTS.md) | 2026-10-01 through 2026-11-05 | Historical results, with dates, baselines and host limits per section. |
| [MCP OAuth checks](MCP_OAUTH_2026-11-03.md) | 2026-11-03 | Windows authorization, refresh and discovery for four providers. |
