import { test } from "node:test";
import assert from "node:assert/strict";
import { channel } from "node:diagnostics_channel";
import { observePreparation, sendObserved } from "../src/context/request-observation.ts";
import { inputMeter } from "../src/context/measurement.ts";
import { policyForContextWindow } from "../src/context/policy.ts";
import { CONTEXT_DIAGNOSTIC_CHANNEL, safeDiagnostic } from "../src/context/diagnostics.ts";
import type { ContextDiagnostic, RequestDiagnostic } from "../src/context/diagnostics.ts";
import { provider } from "../dev/test-support/app.ts";
import { postSse } from "../src/providers/http.ts";
import { normalizeProviderError } from "../src/providers/failure.ts";

test("provider observations include limits and timings for completed, failed, and cancelled sends", async () => {
  const records: ContextDiagnostic[] = [];
  const receive = (value: unknown) => { const e = safeDiagnostic(value); if (e) records.push(e); };
  const source = channel(CONTEXT_DIAGNOSTIC_CHANNEL);
  source.subscribe(receive);
  const request = { model: "fake-1", effort: "high", system: "private instructions", tools: [], messages: [], maxTokens: 1024 };
  const measured = await inputMeter(provider()).measure(request);
  const policy = policyForContextWindow({ tokens: 64_000 }, 85);
  let streams = 0;
  try {
    await sendObserved(provider(), { ...request, onStream: () => streams++ }, measured, policy, 12, 0);
    assert.equal(streams, 1);
    const failed = new Error("private failure");
    await assert.rejects(sendObserved({ ...provider(), async send() { throw failed; } },
      request, measured, policy, 4, 2), (e) => e === failed);
    const control = new AbortController();
    await assert.rejects(sendObserved({ ...provider(), async send() { control.abort(failed); throw failed; } },
      { ...request, signal: control.signal }, measured, policy, 5, 0), (e) => e === failed);
    const events = records as RequestDiagnostic[];
    assert.deepEqual(events.map(e => e.outcome), ["completed", "failed", "cancelled"]);
    assert.equal(events[0]?.windowTokens, 64_000);
    assert.equal(events[0]?.triggerTokens, 54_400);
    assert.equal(events[0]?.requestLimitTokens, 60_800);
    assert.equal(events[0]?.outputBudgetTokens, 1024);
    assert.equal(events[0]?.preparationMs, 12);
    assert.ok(events.every(e => typeof e.providerMs === "number" && e.providerMs >= 0));
    assert.ok(typeof events[0]?.firstEventMs === "number");
    assert.equal(events[1]?.firstEventMs, undefined);
    assert.equal(events[1]?.clippedResults, 2);
    assert.doesNotMatch(JSON.stringify(records), /private/);
    for (const changed of [{ providerMs: -1 }, { windowTokens: Infinity }, { outcome: "secret" }]) {
      assert.equal(safeDiagnostic({ ...events[0], ...changed }), undefined);
    }
  } finally { source.unsubscribe(receive); }
});

test("preparation failures and cancellation are recorded before a provider request exists", async () => {
  const records: ContextDiagnostic[] = [];
  const receive = (value: unknown) => { const e = safeDiagnostic(value); if (e) records.push(e); };
  const source = channel(CONTEXT_DIAGNOSTIC_CHANNEL);
  source.subscribe(receive);
  try {
    const policy = policyForContextWindow({ tokens: 64_000 }, 85);
    const reason = new Error("private preparation error");
    const control = new AbortController();
    for (const abort of [false, true]) {
      await assert.rejects(observePreparation(policy, "budget", control.signal, async () => {
        if (abort) control.abort(reason);
        throw reason;
      }), (e) => e === reason);
    }
    assert.deepEqual(records.map(r => [r.kind, "outcome" in r && r.outcome]),
      [["preparation", "failed"], ["preparation", "cancelled"]]);
    assert.doesNotMatch(JSON.stringify(records), /private/);
  } finally { source.unsubscribe(receive); }
});

test("request diagnostics distinguish transport, visible output, and cached or reasoning usage", async () => {
  const records: ContextDiagnostic[] = [];
  const receive = (value: unknown) => { const event = safeDiagnostic(value); if (event) records.push(event); };
  const source = channel(CONTEXT_DIAGNOSTIC_CHANNEL);
  source.subscribe(receive);
  try {
    const request = { model: "fake", effort: "high", system: "", tools: [], messages: [], maxTokens: 1024 };
    const p = { ...provider(), async send(req: import("../src/types.ts").SendRequest) {
      req.onTransport?.({ transport: "websocket", connectMs: 12, requestBytes: 512,
        reused: true, incremental: true, fallback: false });
      req.onStream?.({ kind: "thinking", text: "private thought" });
      req.onStream?.({ kind: "text", text: "private answer" });
      return { role: "assistant" as const, content: [], usage: { inputTokens: 1000, outputTokens: 40,
        cachedInputTokens: 900, cacheWriteInputTokens: 0, reasoningTokens: 30 } };
    } };
    const measured = await inputMeter(p).measure(request);
    await sendObserved(p, request, measured, policyForContextWindow({ tokens: 64_000 }, 85), 1, 0);
    const event = records[0] as RequestDiagnostic;
    assert.equal(event.transport, "websocket");
    assert.equal(event.requestBytes, 512);
    assert.equal(event.incremental, true);
    assert.equal(event.reportedInputTokens, 1000);
    assert.equal(event.cachedInputTokens, 900);
    assert.equal(event.outputTokens, 40);
    assert.equal(event.reasoningTokens, 30);
    assert.ok(event.firstTextMs! >= event.firstThinkingMs!);
    assert.doesNotMatch(JSON.stringify(event), /private/);
    assert.deepEqual(safeDiagnostic({ ...event, headers: { authorization: "secret" } }), event);
    for (const invalid of [{ transport: "private" }, { requestBytes: -1 }, { cachedInputTokens: NaN },
      { incremental: "yes" }, { firstTextMs: -1 }, { connectMs: "private" }]) {
      assert.equal(safeDiagnostic({ ...event, ...invalid }), undefined);
    }
  } finally { source.unsubscribe(receive); }
});

test("failed HTTP sends retain a safe native code through provider normalization without replay", async (context) => {
  const records: RequestDiagnostic[] = [];
  const receive = (value: unknown) => {
    const event = safeDiagnostic(value);
    if (event?.kind === "request") records.push(event);
  };
  const source = channel(CONTEXT_DIAGNOSTIC_CHANNEL);
  source.subscribe(receive);
  context.after(() => source.unsubscribe(receive));
  const socketError = Object.assign(new Error("private host and address"), { code: "ECONNRESET" });
  const fetchError = new TypeError("fetch failed", { cause: socketError });
  let calls = 0;
  context.mock.method(globalThis, "fetch", async () => { calls++; throw fetchError; });
  const request = { model: "fake", effort: "high", system: "private prompt", tools: [], messages: [], maxTokens: 1024 };
  const p = { ...provider(), async send() {
    try { await postSse("https://private.example/responses", { authorization: "private token" }, {}, 1024); }
    catch (error) {
      assert.ok(error instanceof Error);
      assert.equal(error.cause, fetchError);
      assert.equal(Object.prototype.propertyIsEnumerable.call(error, "cause"), false);
      throw normalizeProviderError("openai-codex", error);
    }
    throw new Error("unexpected response");
  } };
  const measured = await inputMeter(p).measure(request);
  const policy = policyForContextWindow({ tokens: 64_000 }, 85);
  await assert.rejects(sendObserved(p, request, measured, policy, 0, 0), { kind: "network" });
  assert.equal(calls, 1);
  assert.equal(records[0]?.outcome, "failed");
  assert.equal(records[0]?.networkCode, "ECONNRESET");
  assert.equal(records[0]?.firstEventMs, undefined);
  assert.doesNotMatch(JSON.stringify(records), /private|fetch failed|cause|authorization/);
  assert.equal(safeDiagnostic({ ...records[0], networkCode: "private-token" }), undefined);
  const control = new AbortController();
  const cancelled = { ...p, async send() { control.abort(); throw fetchError; } };
  await assert.rejects(sendObserved(cancelled, { ...request, signal: control.signal }, measured, policy, 0, 0));
  assert.equal(records[1]?.outcome, "cancelled");
  assert.equal(records[1]?.networkCode, undefined);
});
