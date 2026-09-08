# Record context behavior

Use the production TUI with an opt-in diagnostic subscriber when investigating
context pressure, unexpected compaction, or differences between estimates and
provider-reported counts. This development launcher is not part of the npm CLI.

The [tokenizer reference](TOKENIZER.md) documents vocabulary provenance and the
independent fixture corpus; [context management](../../docs/CONTEXT.md) describes
measurement, calibration, and compaction behavior.

From the workspace being tested, invoke the checkout's launcher. Normal launch
arguments still apply; `-c` resumes the latest conversation in that workspace:

```powershell
node C:\Users\giova\Codex\jecode\dev\context\record.ts -c
```

The command does not create a synthetic conversation or select a provider. It
starts the regular interactive path with the same settings and permissions.
Record the checkout commit and provider/model separately. An ordinary `jecode`
launch records nothing. Use an isolated development `JECODE_HOME` for destructive
recovery tests as required by the [validation protocol](../validation/README.md).

## Evidence and bounds

Each invocation exclusively creates one `context-*.jsonl` file under
`~/.jecode/diagnostics/` (or the isolated development home). The path and recording
totals are printed after the terminal is restored. Files can be inspected during
the run; ignore an incomplete last line after an abrupt stop. Normal close drains
pending writes and appends an `end` record.

- Request records contain local estimate, chosen input budget, calibration source,
  reference-tokenizer/heuristic method, and returned provider input tokens when
  present. They also contain the resolved window, compaction trigger, safe request
  limit, output budget, and number of tool results shortened to fit. Cached tokens
  remain part of input usage; these records are not billing calculations.
  `outputTokens`, `reasoningTokens`, `cachedInputTokens`, and
  `cacheWriteInputTokens` preserve normalized provider usage when returned.
  A zero cache counter can also mean the adapter did not receive that field;
  it does not prove that the provider performed no caching.
- `preparationMs` covers local preparation before the send, including any
  compaction; `providerMs` covers the adapter call, including internal transport
  retries. `firstEventMs` measures the first text, thinking, or tool stream event
  from that call; it is absent if no such event arrived. These are not terminal
  rendering or input-latency measurements. A failed or cancelled send also emits
  a request record. `completed` means the adapter returned, not that the turn or
  its checkpoint succeeded. Older recordings omit these optional fields.
- `firstTextMs` and `firstThinkingMs` separately measure the first displayed
  text and thinking events at the adapter boundary. They do not reveal hidden
  reasoning timing or provider-side token generation speed.
- `transport` distinguishes HTTP and WebSocket; `connectMs` is elapsed time
  until HTTP response headers or the WebSocket request send, respectively.
  HTTP includes request upload and any internal rejection retry; WebSocket
  includes channel establishment and local request preparation, without waiting
  for model output. These are different boundaries, not comparable server TTFT.
  `requestBytes` measures the uncompressed JSON of the latest observed send.
  `reused` and `incremental` describe socket/prefix reuse; `fallback` identifies
  HTTP after an unsuccessful upgrade or a full retry after a missing predecessor.
  These optional fields omit failed attempts that never reached their observation
  boundary and do not sum all network bytes or retries.
- Failed requests can include `networkCode`, a fixed allowlist of native DNS,
  connection, timeout, and TLS error codes. The first recognized code in a
  bounded cause chain is retained; messages, addresses, and certificate details
  are never recorded. Missing codes mean the cause was unavailable or outside
  the allowlist, not that the network was healthy. Cancellation omits this field.
- WebSocket failures include `transportFailure`: connection failure, peer closure,
  idle/progress timeout, unavailable channel, malformed JSON, or event/queue/stream
  size rejection. `webSocketCloseCode` is included when a close event supplies it;
  close reasons remain private. Native errors may precede the close event, so its
  absence does not establish a clean or unclean close. Local protocol/size failures
  keep their specific UI message instead of appearing as connectivity failures.
- On WebSocket settlement, `receivedEvents`, `receivedChars`, and
  `largestEventChars` describe decoded messages received for the latest send,
  including queued or rejected text events. Characters are UTF-16 code units;
  these counters are not wire bytes or token counts. Non-text messages contribute
  one event and zero characters. They reset when a new socket request is sent.
- `connectionAgeMs` measures elapsed time since the socket opened;
  `lastMessageAgeMs` measures time since its latest decoded message and is absent
  before the first message of each request. Both stop advancing at channel
  failure. `socketReadEnded` records observed TCP EOF; `nativeWebSocketError`
  records a native error before local teardown. Node versions may emit a native
  error for an unframed EOF too. These observations distinguish symptoms, not
  the responsible server, proxy, network, or parser. No ping payload, address,
  close reason, or native error text is recorded.
- `responseStage` records the furthest recognized phase of the latest WebSocket
  response: `awaiting`, `accepted`, `output`, or `terminal`. Opaque reasoning and
  output-item events count as output even without visible text; a terminal error
  is not a successful completion. Unknown peer event names cannot enter this
  field. It resets for each send, does not record content, and does not authorize
  retries. Missing first-text/thinking timings alone do not prove that generation
  had not started. Older recordings do not contain this optional field.
- Preparation failures before a send emit `preparation` records with numeric
  limits, elapsed time, and failed/cancelled outcome. Early capacity-discovery
  cancellation and individual HTTP retries are outside this recorder's boundary.
- Compaction records contain budget/overflow/manual cause, outcome, before/after
  counts when available, resolved limits, and total local-plus-provider duration. `no-prefix` means
  planning found no eligible prefix; `cancelled` and `timeout` remain distinct.
  `incomplete` means the summary response was truncated, refused, otherwise
  unfinished, or contained unexpected tool blocks; no new anchor was accepted.
  A zero `beforeTokens` means an internal caller did not supply a measurement.
- Once a summary send starts, compaction records also include `summaryChars`
  (streamed UTF-16 code units) and `summaryProviderMs`. `firstSummaryTextMs` is
  absent when no nonempty summary text arrived. These distinguish a silent wait
  from a still-growing summary that reaches the deadline; no summary text is
  recorded. Older traces omit these fields.
- Records contain no prompts, summaries, file paths, tool arguments, output,
  credentials, account IDs, provider raw data, or arbitrary error messages.
- Each file accepts at most 4,096 events plus the end record. The asynchronous
  writer queues at most 64 pending events. Excess events are counted as dropped;
  they never delay model/tool execution. A dropped record prevents a claim that
  the trace is complete. I/O failure stops recording and is reported on close.
- Files use exclusive creation and owner-only permissions where supported; the
  output directory must be a direct, anchored directory. The channel subscriber
  whitelists fields even when a publisher supplies additional properties.

An accepted summary is not proof of a committed checkpoint. Session validation
is still needed to confirm the durable anchor. The trace also does not establish
summary quality, display latency, or accessibility. Capture those observations
using the release-candidate protocol.

Keep recordings local and outside version control. Retain the sanitized evidence
needed for review, then remove obsolete recordings through the
[temporary-output lifecycle](../README.md#temporary-outputs).
