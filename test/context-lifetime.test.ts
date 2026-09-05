import { test } from "node:test";
import assert from "node:assert/strict";
import { inputLifetime } from "../src/context/lifetime.ts";
import { inputMeter } from "../src/context/measurement.ts";
import { provider } from "../dev/test-support/app.ts";
import { deferred } from "../dev/test-support/app-harness.ts";
import type { RequestInput } from "../src/types.ts";

const input: RequestInput = { model: "fake-1", effort: "high", system: "instructions", tools: [],
  messages: [{ role: "user", content: [{ kind: "text", text: "source ".repeat(1000) }] }] };

test("conversation lifetime retains observations and isolates providers, resets, and resumes", async () => {
  const lifetime = inputLifetime();
  const from = provider();
  const first = lifetime.forTurn(from, "conversation-1");
  first.observe(await first.measure(input), 100);
  assert.equal((await lifetime.forTurn(from, "conversation-1").measure(input)).inputTokens, 100);
  for (const changed of [lifetime.forTurn(provider(), "conversation-1"),
    lifetime.forTurn(from, "conversation-2"), inputLifetime().forTurn(from, "conversation-1")]) {
    assert.equal((await changed.measure(input)).source, "estimate");
  }
  const selected = lifetime.forTurn(from, "conversation-1");
  const pending = await selected.measure(input);
  lifetime.reset();
  selected.observe(pending, 100);
  assert.equal((await selected.measure(input)).source, "estimate");
  assert.equal((await lifetime.forTurn(from, "conversation-1").measure(input)).source, "estimate");
});

test("reset during measurement rejects its later provider observation", async () => {
  const started = deferred();
  const release = deferred();
  const meter = inputMeter({ ...provider(), async measureInput() {
    started.release(); await release.wait; return 1000;
  } });
  const pending = meter.measure(input);
  await started.wait;
  meter.reset();
  release.release();
  meter.observe(await pending, 100);
  assert.equal((await meter.measure(input)).inputTokens, 1000);
});

test("an invalidated prefix cannot regain its old observation by reverting the change", async () => {
  const meter = inputMeter(provider());
  meter.observe(await meter.measure(input), 100);
  await meter.measure({ ...input, effort: "low" });
  assert.equal((await meter.measure(input)).source, "estimate");
});
