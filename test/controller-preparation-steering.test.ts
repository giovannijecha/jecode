import { test } from "node:test";
import assert from "node:assert/strict";
import { runTurn } from "../src/controller.ts";
import { policyForContextWindow } from "../src/context/policy.ts";
import { steeringInbox } from "../src/steering.ts";
import type { Message } from "../src/types.ts";
import { assistantText, events, options, scripted, texts } from "../dev/test-support/controller.ts";

const user = (text: string): Message => ({ role: "user", content: [{ kind: "text", text }] });

for (const phase of ["metadata", "compaction"] as const) {
  test(`guidance arriving during ${phase} reaches the first generation`, async () => {
    const inbox = steeringInbox();
    const provider = scripted([assistantText("Revised result.")]);
    const history = [user("Original requirement.")];
    const sink = events();
    const received: string[] = [];
    let compactions = 0;
    sink.onSteering = text => received.push(text);
    sink.onContext = async () => {
      if (phase !== "compaction") return undefined;
      compactions++;
      await new Promise<void>(resolve => setImmediate(resolve));
      inbox.offer("Preserve the old API.");
      return [user("Compacted working memory.")];
    };
    await runTurn(history, options(provider, { steering: inbox, contextPolicy: async () => {
      if (phase === "metadata") {
        await new Promise<void>(resolve => setImmediate(resolve));
        inbox.offer("Preserve the old API.");
      }
      return policyForContextWindow(undefined, 85);
    } }), sink);
    assert.equal(provider.seen.length, 1, "no avoidable response before processing guidance");
    assert.deepEqual(texts(provider.seen[0]!.messages), [
      phase === "metadata" ? "Original requirement." : "Compacted working memory.",
      "Preserve the old API.",
    ]);
    assert.deepEqual(texts(history), ["Original requirement.", "Preserve the old API.", "Revised result."]);
    assert.deepEqual(received, ["Preserve the old API."]);
    assert.equal(compactions, phase === "compaction" ? 1 : 0);
  });
}

test("guidance arriving while the revised request is measured is included and rebudgeted", async () => {
  const inbox = steeringInbox();
  const provider = scripted([assistantText("Done.")]);
  let measurements = 0;
  provider.measureInput = async (request) => {
    measurements++;
    if (measurements === 1) inbox.offer("First correction.");
    if (measurements === 2) inbox.offer("Second correction.");
    return 100 + request.messages.length * 100;
  };
  const reported: number[] = [];
  const sink = events();
  sink.onRequestInput = n => reported.push(n);
  await runTurn([user("Start.")], options(provider, { steering: inbox }), sink);
  assert.equal(provider.seen.length, 1);
  assert.deepEqual(texts(provider.seen[0]!.messages), ["Start.", "First correction.", "Second correction."]);
  assert.ok(measurements >= 3);
  assert.ok(reported[0]! >= 400, "the sent context includes both corrections in its budget");
});

test("oversized late guidance is retained without sending an over-budget request", async () => {
  const inbox = steeringInbox();
  const provider = scripted([]);
  const history = [user("Start.")];
  const guidance = "\u{10ffff}".repeat(10_000);
  const sink = events();
  sink.onContext = async () => { inbox.offer(guidance); return undefined; };
  await assert.rejects(runTurn(history, options(provider, {
    steering: inbox, contextPolicy: async () => policyForContextWindow({ tokens: 4_096 }, 85),
  }), sink), /request input needs approximately/);
  assert.equal(provider.seen.length, 0);
  assert.deepEqual(texts(history), ["Start.", guidance]);
  assert.equal(inbox.pending, 0);
});

test("cancelled preparation leaves undelivered guidance queued", async () => {
  const inbox = steeringInbox();
  const provider = scripted([]);
  const control = new AbortController();
  const failure = new Error("interrupted preparation");
  const history = [user("Start.")];
  const sink = events();
  sink.onContext = async () => { inbox.offer("Keep this correction."); control.abort(failure); return undefined; };
  await assert.rejects(runTurn(history, options(provider, { steering: inbox }), sink, control.signal), e => e === failure);
  assert.equal(provider.seen.length, 0);
  assert.deepEqual(texts(history), ["Start."]);
  assert.deepEqual(inbox.close(), ["Keep this correction."]);
});

test("late guidance alone cannot authorize replay of a context-rejected request", async () => {
  const inbox = steeringInbox();
  const failure = Object.assign(new Error("context window exceeded"), { status: 400 });
  let requests = 0;
  const provider = { ...scripted([]), async send(): Promise<Message> { requests++; throw failure; } };
  const sink = events();
  sink.onContext = async (_canonical, _context, request) => {
    if (request.reason === "overflow") inbox.offer("New correction.");
    return undefined;
  };
  await assert.rejects(runTurn([user("Start.")], options(provider, { steering: inbox }), sink), e => e === failure);
  assert.equal(requests, 1);
  assert.deepEqual(inbox.close(), ["New correction."]);
});
