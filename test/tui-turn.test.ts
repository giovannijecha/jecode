import { test } from "node:test";
import assert from "node:assert/strict";
import { renderAll } from "../src/tui/blocks.ts";
import { STEEL } from "../src/ui/theme.ts";
import { stage, callOf, strip } from "../dev/test-support/tui.ts";

test("transcription tracks the live activity phase", () => {
  const log: string[] = [];
  const { events } = stage(log);
  const call = callOf("1", "run_command", { command: "node run.js" });

  events.onStream({ kind: "thinking", text: "hm" });
  events.onStream({ kind: "text", text: "ok" });
  events.onStream({ kind: "tool", name: "run_command" });
  events.onToolPreparing?.(call, 2, 3);
  events.onToolCall(call);
  events.onToolStart?.(call, 2, 3);
  events.onToolResult(call, { kind: "tool_result", id: "1", output: "11", isError: false }, "exit 0");

  assert.deepEqual(log, [
    "Thinking",
    "Responding",
    "Preparing run_command",
    "Preparing run_command · tool 2/3",
    "Running run_command · tool 2/3",
    "Waiting",
  ]);
  assert.doesNotMatch(log.join("\n"), /\bstep\b/);
});

test("a tool rail starts timing only when execution actually begins", () => {
  const { blocks, events } = stage([]);
  const call = callOf("1", "read_file", { path: "a.ts" });

  events.onToolPreparing?.(call, 1, 1);
  events.onToolCall(call);
  const block = blocks[0];
  assert.equal(block?.kind === "tool" ? block.right : undefined, "ready");
  assert.equal(block?.kind === "tool" ? block.startedAt : undefined, undefined);

  events.onToolStart?.(call, 1, 1);
  assert.equal(block?.kind === "tool" ? block.right : undefined, "running");
  assert.equal(typeof (block?.kind === "tool" ? block.startedAt : undefined), "number");

  events.onToolResult(call, { kind: "tool_result", id: "1", output: "ok", isError: false }, "1 line");
  assert.equal(block?.kind === "tool" ? block.startedAt : undefined, undefined);
  assert.equal(typeof (block?.kind === "tool" ? block.durationMs : undefined), "number");
});

test("reasoning changes from live to retained preview at the next stream kind", () => {
  const { blocks, events } = stage([]);
  events.onStream({ kind: "thinking", text: "inspect" });
  const reasoning = blocks[0];
  assert.equal(reasoning?.kind, "reasoning");
  if (reasoning?.kind !== "reasoning") return;
  assert.equal(reasoning.live, true);
  assert.equal(reasoning.expanded, false);

  events.onStream({ kind: "text", text: "done" });
  assert.equal(reasoning.live, false);
});

test("a terminal escape split across stream chunks remains inert", () => {
  const { blocks, events } = stage([]);
  const escape = String.fromCharCode(27);
  const bell = String.fromCharCode(7);
  events.onStream({ kind: "text", text: escape });
  events.onStream({ kind: "text", text: `]52;c;payload${bell}` });

  const drawn = strip(renderAll(blocks, 60, STEEL)).join("\n");
  assert.equal(drawn.includes(escape), false);
  assert.ok(drawn.includes(`${String.fromCodePoint(0x241b)}]52`));
});

test("a reasoning-only stream is sealed when transcription finishes", () => {
  const { blocks, events } = stage([]);
  events.onStream({ kind: "thinking", text: "inspect" });
  events.finish();
  const reasoning = blocks[0];
  assert.equal(reasoning?.kind === "reasoning" ? reasoning.live : undefined, false);
});

test("finishing an interrupted transcription settles its running tool", () => {
  const { blocks, events } = stage([]);
  events.onToolCall(callOf("1", "run_command", { command: "npm test" }));

  events.finish("interrupted");

  const tool = blocks[0];
  assert.equal(tool?.kind === "tool" ? tool.tone : undefined, "fail");
  assert.equal(tool?.kind === "tool" ? tool.right : undefined, "interrupted");
  assert.equal(tool?.kind === "tool" ? tool.startedAt : undefined, undefined);
});

test("live command output updates the existing pending rail", () => {
  const { blocks, events } = stage([]);
  const call = callOf("1", "run_command", { command: "many" });
  events.onToolCall(call);
  events.onToolStart?.(call, 1, 1);
  events.onToolOutput?.(call, Array.from({ length: 12 }, (_, index) => `line ${index}`).join("\n"));

  const block = blocks[0];
  assert.equal(block?.kind, "tool");
  if (block?.kind !== "tool") return;
  assert.equal(block.body?.length, 12);
  const shown = strip(renderAll([block], 60, STEEL, { now: block.startedAt })).join("\n");
  assert.match(shown, /8 earlier lines · ctrl\+o/);
  assert.match(shown, /line 8/);
  assert.match(shown, /line 11/);
  assert.doesNotMatch(shown, /line 0/);
});
