import { test } from "node:test";
import assert from "node:assert/strict";
import type { Block } from "../src/tui/blocks.ts";
import { render } from "../src/tui/blocks.ts";
import { transcriptRenderer } from "../src/tui/transcript-view.ts";
import { STEEL } from "../src/ui/theme.ts";

test("an unchanged long transcript reuses every rendered block", () => {
  const blocks: Block[] = Array.from(
    { length: 1_000 },
    (_, index) => ({ kind: "answer", text: `answer ${index}` }),
  );
  const calls = new Map<Block, number>();
  const transcript = transcriptRenderer((block, width, palette) => {
    calls.set(block, (calls.get(block) ?? 0) + 1);
    return render(block, width, palette);
  });

  const first = transcript.viewport(blocks, 80, 20, 0, STEEL);
  const second = transcript.viewport(blocks, 80, 20, 0, STEEL);

  assert.deepEqual(second, first);
  assert.equal([...calls.values()].reduce((sum, count) => sum + count, 0), blocks.length);
  assert.ok(first.maxScroll > 0);
  assert.match(first.rows.join("\n"), /answer 999/);
  assert.doesNotMatch(first.rows.join("\n"), /answer 0(?:\D|$)/);
});

test("streaming invalidates only the block whose visible state changed", () => {
  const settled: Block = { kind: "answer", text: "kept" };
  const live: Block = { kind: "reasoning", text: "first", live: true };
  const blocks = [settled, live];
  const calls = new Map<Block, number>();
  const transcript = transcriptRenderer((block, width, palette) => {
    calls.set(block, (calls.get(block) ?? 0) + 1);
    return render(block, width, palette);
  });

  transcript.viewport(blocks, 60, 12, 0, STEEL);
  live.text += " second";
  transcript.invalidate(live);
  transcript.viewport(blocks, 60, 12, 0, STEEL);

  assert.equal(calls.get(settled), 1);
  assert.equal(calls.get(live), 2);
});

test("width changes invalidate cached rows while viewport scrolling does not", () => {
  const block: Block = { kind: "answer", text: "one two three four five six seven eight" };
  const blocks = [block];
  let calls = 0;
  const transcript = transcriptRenderer((current, width, palette) => {
    calls++;
    return render(current, width, palette);
  });

  transcript.viewport(blocks, 20, 2, 0, STEEL);
  transcript.viewport(blocks, 20, 2, 1, STEEL);
  transcript.viewport(blocks, 30, 2, 0, STEEL);

  assert.equal(calls, 2);
});

test("appending renders only the new tail block", () => {
  const blocks: Block[] = [{ kind: "answer", text: "first" }];
  const calls = new Map<Block, number>();
  const transcript = transcriptRenderer((block, width, palette) => {
    calls.set(block, (calls.get(block) ?? 0) + 1);
    return render(block, width, palette);
  });

  transcript.viewport(blocks, 40, 8, 0, STEEL);
  const added: Block = { kind: "answer", text: "second" };
  blocks.push(added);
  const viewport = transcript.viewport(blocks, 40, 8, 0, STEEL);

  assert.equal(calls.get(blocks[0] as Block), 1);
  assert.equal(calls.get(added), 1);
  assert.match(viewport.rows.join("\n"), /first[\s\S]*second/);
});

test("a dirty block that changes height updates cumulative viewport offsets", () => {
  const first: Block = { kind: "answer", text: "short" };
  const last: Block = { kind: "answer", text: "tail" };
  const blocks = [first, last];
  const transcript = transcriptRenderer();
  const before = transcript.viewport(blocks, 16, 3, 0, STEEL);

  first.text = "one two three four five six seven eight nine ten";
  transcript.invalidate(first);
  const after = transcript.viewport(blocks, 16, 3, 0, STEEL);

  assert.ok(after.maxScroll > before.maxScroll);
  assert.match(after.rows.join("\n"), /tail/);
});
