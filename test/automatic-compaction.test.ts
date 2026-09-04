import { test } from "node:test";
import assert from "node:assert/strict";
import {
  automaticCompactionGate,
  automaticCompactionKey,
} from "../src/context/automatic.ts";

test("automatic compaction never retries one failed context generation", () => {
  const gate = automaticCompactionGate();
  const failed = automaticCompactionKey("openai", "model", 7, 12);
  const budget = { key: failed, reason: "budget" } as const;

  assert.equal(gate.allows(budget), true);
  gate.failed(budget);
  assert.equal(gate.allows(budget), false);
  assert.equal(gate.allows({ key: failed, reason: "overflow" }), true);

  const changed = automaticCompactionKey("openai", "model", 7, 13);
  assert.equal(gate.allows({ key: changed, reason: "budget" }), true);
  assert.equal(gate.allows({ key: changed, reason: "overflow" }), true);
});

test("automatic compaction settles only one generation and reset clears it", () => {
  const gate = automaticCompactionGate();
  const key = automaticCompactionKey("anthropic", "model", 1, 4);
  const budget = { key, reason: "budget" } as const;

  gate.succeeded(budget);
  assert.equal(gate.allows(budget), false);
  assert.equal(gate.allows({ key, reason: "overflow" }), true);
  const next = automaticCompactionKey("anthropic", "model", 1, 5);
  assert.equal(gate.allows({ key: next, reason: "budget" }), true);
  assert.equal(gate.allows({ key: next, reason: "overflow" }), true);

  gate.failed({ key, reason: "overflow" });
  gate.reset();
  assert.equal(gate.allows({ key, reason: "budget" }), true);
});
