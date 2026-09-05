import { test } from "node:test";
import assert from "node:assert/strict";
import { tuiHost } from "../dev/benchmarks/tui-host.ts";
import { measuredOutput } from "../dev/benchmarks/tui-output.ts";
import { session } from "../dev/test-support/app.ts";

test("the counting sink reports UTF-8 bytes and drains a pressured consumer", async () => {
  const output = measuredOutput(1_000_000);
  const text = "日".repeat(2_000);
  let acknowledged = 0;
  try {
    assert.equal(output.write(text, () => acknowledged++), 6_000);
    output.write("next", () => acknowledged++);
    assert.deepEqual(output.stats(), { bytes: 6_004, writes: 2, peakQueuedBytes: 6_004, backpressureWrites: 2 });
    assert.equal(acknowledged, 0);
    await output.close();
    assert.equal(acknowledged, 2);
  } finally {
    output.destroy();
  }
});

test("invalid consumer rates fail before starting the output sink", () => {
  for (const rate of [0, -1, NaN, Infinity]) assert.throws(() => measuredOutput(rate), /positive finite/);
});

test("the benchmark waits for matching production frames and reset through the public input path", async () => {
  const current = session();
  const host = await tuiHost(current, { cols: 60, rows: 20 });
  try {
    const input = await host.measure(() => host.input("measured input"), (text, cursor) =>
      text.includes("measured input") && cursor !== undefined);
    assert.ok(input.bytes > 0);
    assert.ok(input.milliseconds >= input.paintMilliseconds);
    await host.measure(() => host.input("\u0015/settings\r"), (text) => text.includes("● model"));
    await host.measure(() => host.input("\u001b[B"), (text) => text.includes("● effort"));
    await host.measure(() => host.input("\u001b"), (_text, cursor) => cursor !== undefined);
    await host.measure(() => host.input("/new\r"), (text, cursor) =>
      !text.includes("measured input") && cursor !== undefined);
    await host.close();
    assert.equal(current.conversation.nodes.length, 0);
    assert.ok(input.acknowledgedMilliseconds !== undefined);
  } catch (error) {
    await host.close(false);
    throw error;
  }
});

test("closing the measurement host cancels a pending observation", async () => {
  const host = await tuiHost(session(), { cols: 60, rows: 20 });
  const pending = host.measure(() => host.input("waiting"), () => false);
  const rejected = assert.rejects(pending, /benchmark host closed/);
  await host.close(false);
  await rejected;
});
