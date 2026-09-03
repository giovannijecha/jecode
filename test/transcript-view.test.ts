import { test } from "node:test";
import assert from "node:assert/strict";
import type { Block } from "../src/tui/blocks.ts";
import { render, renderAll } from "../src/tui/blocks.ts";
import { transcriptRenderer } from "../src/tui/transcript-view.ts";
import { STEEL } from "../src/ui/theme.ts";
import { textWidth } from "../src/ui/width.ts";

test("a cold long transcript paints its tail before bounded background reflow", () => {
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
  const firstFrames = [...calls.values()].reduce((sum, count) => sum + count, 0);
  assert.ok(firstFrames > 0);
  assert.ok(firstFrames < blocks.length);
  assert.equal(first.pending, true);
  assert.ok(first.maxScroll > 0);
  assert.match(first.rows.join("\n"), /answer 999/);
  assert.doesNotMatch(first.rows.join("\n"), /answer 0(?:\D|$)/);

  let settled = second;
  while (settled.pending) settled = transcript.viewport(blocks, 80, 20, 0, STEEL);
  const completeCalls = [...calls.values()].reduce((sum, count) => sum + count, 0);
  assert.equal(completeCalls, blocks.length);

  transcript.viewport(blocks, 80, 20, 80, STEEL);
  assert.equal([...calls.values()].reduce((sum, count) => sum + count, 0), completeCalls);
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

test("live tool motion repaints locally, settles, and then leaves a stable cache", () => {
  let now = 1_000;
  const tool: Block = {
    kind: "tool",
    name: "run_command",
    target: "npm test",
    right: "running",
    tone: "pending",
    startedAt: now,
    body: [{ kind: "out", text: "starting" }],
  };
  const blocks = [tool];
  const transcript = transcriptRenderer(render, () => now);

  transcript.invalidate(tool);
  const first = transcript.viewport(blocks, 100, 8, 0, STEEL, { now });
  now += 80;
  const second = transcript.viewport(blocks, 100, 8, 0, STEEL, { now });

  assert.equal(first.animating, true);
  assert.equal(second.animating, true);
  assert.notEqual(first.rows.join("\n"), second.rows.join("\n"));

  if (tool.kind !== "tool") return;
  tool.tone = "ok";
  tool.right = "exit 0";
  tool.durationMs = now - (tool.startedAt ?? now);
  tool.startedAt = undefined;
  transcript.invalidate(tool);
  assert.equal(transcript.viewport(blocks, 100, 8, 0, STEEL, { now }).animating, true);

  now = 2_600;
  const resting = transcript.viewport(blocks, 100, 8, 0, STEEL, { now });
  const cached = transcript.viewport(blocks, 100, 8, 0, STEEL, { now });
  assert.equal(resting.animating, false);
  assert.deepEqual(cached, resting);
});

test("historical and reduced-motion tools render directly at rest", () => {
  let now = 1_000;
  const historical: Block = {
    kind: "tool",
    name: "read_file",
    target: "README.md",
    right: "40 lines",
    tone: "ok",
    durationMs: 7,
  };
  const history = transcriptRenderer(render, () => now);
  history.viewport([historical], 80, 6, 0, STEEL, { now });
  historical.expanded = true;
  history.invalidate(historical);
  assert.equal(history.viewport([historical], 80, 6, 0, STEEL, { now }).animating, false);

  const pending: Block = {
    kind: "tool",
    name: "run_command",
    target: "npm test",
    right: "running",
    tone: "pending",
    startedAt: now,
  };
  const reduced = transcriptRenderer(render, () => now);
  reduced.invalidate(pending);
  const frame = reduced.viewport([pending], 80, 6, 0, STEEL, {
    now,
    reducedMotion: true,
  });
  assert.equal(frame.animating, false);
  assert.match(frame.rows.join("\n"), /○\s+run_command/);
  assert.doesNotMatch(frame.rows.join("\n"), /[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏]/);
});

test("wide live tool motion stays inside the frame and bounds its moving trail", () => {
  let now = 0;
  const tool: Block = {
    kind: "tool",
    name: "run_command",
    target: "npm test",
    right: "running",
    tone: "pending",
    startedAt: 0,
  };
  const transcript = transcriptRenderer(render, () => now);
  transcript.invalidate(tool);
  now = 750;

  const frame = transcript.viewport([tool], 200, 6, 0, STEEL, { now });
  const toolRow = frame.rows.find((line) => line.includes("run_command")) ?? "";
  const separatorsAndTrail = toolRow.match(/·/gu)?.length ?? 0;

  assert.ok(frame.rows.every((line) => textWidth(line) <= 200));
  assert.ok(separatorsAndTrail > 1, "the travelling trail is visible mid-flight");
  assert.ok(separatorsAndTrail <= 8, "seven trail cells plus the duration separator");
  assert.doesNotMatch(toolRow, /npm test·|·running/);
});

test("two recent widths retain their cached rows while viewport scrolling does not", () => {
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
  transcript.viewport(blocks, 20, 2, 0, STEEL);

  assert.equal(calls, 2);

  transcript.viewport(blocks, 40, 2, 0, STEEL);
  transcript.viewport(blocks, 30, 2, 0, STEEL);
  assert.equal(calls, 4);
});

test("semantic invalidation refreshes the same block at both cached widths", () => {
  const block: Block = { kind: "answer", text: "before" };
  const blocks = [block];
  let calls = 0;
  const transcript = transcriptRenderer((current, width, palette) => {
    calls++;
    return render(current, width, palette);
  });

  transcript.viewport(blocks, 20, 2, 0, STEEL);
  transcript.viewport(blocks, 30, 2, 0, STEEL);
  block.text = "after";
  transcript.invalidate(block);
  const narrow = transcript.viewport(blocks, 20, 2, 0, STEEL);
  const wide = transcript.viewport(blocks, 30, 2, 0, STEEL);

  assert.equal(calls, 4);
  assert.match(narrow.rows.join("\n"), /after/);
  assert.match(wide.rows.join("\n"), /after/);
});

test("a resized large transcript reflows only a bounded first-frame working set", () => {
  const blocks: Block[] = Array.from(
    { length: 20_000 },
    (_, index) => ({ kind: "answer", text: `answer ${index}` }),
  );
  let calls = 0;
  const transcript = transcriptRenderer((block) => {
    calls++;
    return ["", block.kind === "answer" ? block.text : ""];
  });

  let initial = transcript.viewport(blocks, 80, 40, 0, STEEL);
  while (initial.pending) initial = transcript.viewport(blocks, 80, 40, 0, STEEL);
  const beforeResize = calls;

  const resized = transcript.viewport(blocks, 120, 40, 0, STEEL);
  const firstFrameCalls = calls - beforeResize;

  assert.equal(resized.pending, true);
  assert.ok(firstFrameCalls > 0);
  assert.ok(firstFrameCalls < blocks.length / 10);
  assert.match(resized.rows.join("\n"), /answer 19999/);

  const cached = transcript.viewport(blocks, 80, 40, 0, STEEL);
  assert.equal(cached.pending, false);
  assert.equal(calls - beforeResize, firstFrameCalls);
});

test("a scrolled viewport stays exact while background reflow is paused", () => {
  const blocks: Block[] = Array.from(
    { length: 1_000 },
    (_, index) => ({ kind: "answer", text: String(index) }),
  );
  const transcript = transcriptRenderer((block, width) => {
    const index = Number(block.kind === "answer" ? block.text : -1);
    const rows = width === 40 ? index % 7 + 1 : index % 3 + 1;
    return Array.from({ length: rows }, (_, row) => `${index}:${row}`);
  });

  let narrow = transcript.viewport(blocks, 20, 12, 0, STEEL);
  while (narrow.pending) narrow = transcript.viewport(blocks, 20, 12, 0, STEEL);
  const resized = transcript.viewport(blocks, 40, 12, 900, STEEL);
  const shown = resized.rows.map((line) => line.trim()).filter((line) => line !== "");

  assert.equal(resized.pending, false);
  assert.equal(shown.length, 12);
  for (let index = 1; index < shown.length; index++) {
    const [previousBlock, previousRow] = (shown[index - 1] as string).split(":").map(Number);
    const [block, row] = (shown[index] as string).split(":").map(Number);
    assert.ok(
      block === previousBlock && row === previousRow + 1 ||
        block === previousBlock + 1 && row === 0,
      `${shown[index - 1]} must be followed by ${shown[index]}`,
    );
  }

  assert.equal(transcript.viewport(blocks, 40, 12, 0, STEEL).pending, true);
});

test("completed incremental reflow matches the exact eager viewport", () => {
  const blocks: Block[] = Array.from({ length: 300 }, (_, index) =>
    index % 5 === 0
      ? { kind: "user", text: `question ${index} ${"wide ".repeat(index % 9)}` }
      : { kind: "answer", text: `answer ${index} ${"wrapped ".repeat(index % 13)}` }
  );
  const width = 24;
  const height = 11;
  const transcript = transcriptRenderer();
  let viewport = transcript.viewport(blocks, width, height, 0, STEEL);
  while (viewport.pending) viewport = transcript.viewport(blocks, width, height, 0, STEEL);

  const eager = renderAll(blocks, width, STEEL);
  const maxScroll = Math.max(0, eager.length - height);
  for (const requested of [0, 7, Math.floor(maxScroll / 2), maxScroll, maxScroll + 100]) {
    viewport = transcript.viewport(blocks, width, height, requested, STEEL);
    const scroll = Math.min(requested, maxScroll);
    const start = Math.max(0, eager.length - height - scroll);
    const expected = eager.slice(start, start + height);
    while (expected.length < height) expected.unshift("");
    assert.deepEqual(viewport.rows, expected);
    assert.equal(viewport.maxScroll, maxScroll);
    assert.equal(viewport.pending, false);
  }
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
