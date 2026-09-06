import { test } from "node:test";
import assert from "node:assert/strict";
import { assembleOpenAI } from "../src/providers/openai-stream.ts";
import { stage, strip } from "../dev/test-support/tui.ts";
import { renderAll } from "../src/tui/blocks.ts";
import { configureColor } from "../src/ui/render.ts";
import { STEEL } from "../src/ui/theme.ts";
import { textWidth } from "../src/ui/width.ts";
import type { StreamEvent } from "../src/types.ts";

async function* feed(events: unknown[]): AsyncGenerator<unknown> {
  yield* events;
  yield { type: "response.completed", response: { output: [] } };
}

function delta(text: unknown, summary_index?: number, item_id?: string, output_index?: number) {
  return { type: "response.reasoning_summary_text.delta", delta: text, summary_index, item_id, output_index };
}

test("Responses summary parts stay separated through streaming, settlement, and narrow rendering", async () => {
  const { blocks, events } = stage([]);
  const output = [{ type: "reasoning", id: "r1", summary: [
    { type: "summary_text", text: "**Checking state**" },
    { type: "summary_text", text: "**Testing export**" },
  ], encrypted_content: "opaque-fixture" }];
  const result = await assembleOpenAI(feed([
    { type: "response.reasoning_summary_part.added", item_id: "r1", summary_index: 0 },
    delta("**Checking ", 0, "r1"), delta("state**", 0, "r1"),
    { type: "response.reasoning_summary_text.done", item_id: "r1", summary_index: 0 },
    { type: "response.reasoning_summary_part.done", item_id: "r1", summary_index: 0 },
    { type: "response.reasoning_summary_part.added", item_id: "r1", summary_index: 1 },
    delta("**Testing export**", 1, "r1"),
    { type: "response.output_item.done", item: output[0] },
  ]), events.onStream);
  events.finish();
  assert.deepEqual(result.output, output, "display separators never change provider replay data");
  assert.equal(blocks.length, 1);
  const block = blocks[0];
  assert.ok(block?.kind === "reasoning");
  assert.equal(block.text, "**Checking state**\n\n**Testing export**");
  assert.equal(block.live, false);
  for (const color of [true, false]) {
    configureColor(color);
    for (const cols of [20, 60]) {
      const rows = renderAll(blocks, cols, STEEL);
      const plain = strip(rows);
      assert.ok(plain.every(row => textWidth(row) <= cols));
      assert.match(plain.join("\n"), /Checking state\n+Testing export/);
      if (!color) assert.ok(rows.every(row => !row.includes("\u001b")));
    }
  }
  configureColor(true);
});

test("summary identity changes separate parts even without completion events", async () => {
  const streamed: StreamEvent[] = [];
  await assembleOpenAI(feed([
    delta("First", 0, "r1", 0), delta(" heading", 0, "r1", 0),
    delta("Second heading", 1, "r1", 0),
    delta("Third heading", 0, "r2", 1),
    delta("Fourth heading", 0, undefined, 2),
  ]), event => streamed.push(event));
  assert.equal(streamed.filter(e => e.kind === "thinking").map(e => e.text).join(""),
    "First heading\n\nSecond heading\n\nThird heading\n\nFourth heading");
});

test("idless summary boundaries ignore empty deltas and do not add trailing blank rows", async () => {
  const streamed: StreamEvent[] = [];
  await assembleOpenAI(feed([
    delta(""), delta(null), delta("First\n"),
    { type: "response.reasoning_summary_text.done" },
    { type: "response.reasoning_summary_part.done" },
    delta(""), delta("\nSecond"),
    { type: "response.output_item.done", item: { type: "reasoning" } },
    delta("Third"), { type: "response.reasoning_summary_part.added" },
    delta("Fourth"), { type: "response.reasoning_summary_text.done" },
  ]), event => streamed.push(event));
  assert.equal(streamed.filter(e => e.kind === "thinking").map(e => e.text).join(""),
    "First\n\nSecond\n\nThird\n\nFourth");
  assert.equal(streamed.length, 4);
});

test("answers and tools end the preceding summary display group", async () => {
  const { blocks, events } = stage([]);
  await assembleOpenAI(feed([
    delta("First", 0, "r1"), { type: "response.reasoning_summary_text.done" },
    { type: "response.output_text.delta", delta: "Answer" },
    delta("Second", 0, "r2"),
    { type: "response.output_item.added", item: { type: "function_call", id: "fc1", name: "read_file" } },
    delta("Third", 0, "r3"),
  ]), events.onStream);
  events.finish();
  assert.deepEqual(blocks.map(b => "text" in b ? b.text : ""), ["First", "Answer", "Second", "Third"]);
});
