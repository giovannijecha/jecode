import { test } from "node:test";
import assert from "node:assert/strict";
import { channel } from "node:diagnostics_channel";
import { observePreparation, sendObserved } from "../src/context/request-observation.ts";
import { inputMeter } from "../src/context/measurement.ts";
import { policyForContextWindow } from "../src/context/policy.ts";
import { CONTEXT_DIAGNOSTIC_CHANNEL, safeDiagnostic } from "../src/context/diagnostics.ts";
import type { ContextDiagnostic, RequestDiagnostic } from "../src/context/diagnostics.ts";
import { provider } from "../dev/test-support/app.ts";

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
