import { test } from "node:test";
import assert from "node:assert/strict";
import type { Block } from "../src/tui/blocks.ts";
import { transcribe } from "../src/tui/turn.ts";
import * as approve from "../src/tui/approve.ts";
import * as picker from "../src/tui/picker.ts";
import { STEEL } from "../src/ui/theme.ts";
import { callOf, strip } from "../dev/test-support/tui.ts";

test("an approval is a menu of answers, not a key to guess at", () => {
  const prompt = approve.promptFor(callOf("1", "write_file", { path: "package.json" }), "package.json", STEEL);
  const bare = strip(picker.panel(prompt, 80, STEEL));
  const shown = bare.join("\n");

  const question = bare.findIndex((row) => row.includes("Write this file?"));
  const target = bare.findIndex((row) => row.includes("write_file · package.json"));
  assert.ok(question >= 0 && target > question, "the target has its own row beneath the question");
  assert.match(shown, /● Yes, once.*\by\b/);
  assert.match(shown, /Yes, this file for the session/);
  assert.match(shown, /No, and say why/);
  assert.match(shown, /esc deny/);
  const detail = bare.findIndex((row) => row.includes("Approve only this call."));
  assert.match(bare[detail + 1] ?? "", /↑↓ choose/, "one-line approval details touch the controls without an empty row");
  assert.deepEqual(prompt.options.map((option) => option.key), ["y", "a", "n"]);
});

test("the arrows walk the answers and wrap rather than dead-ending", () => {
  const prompt = approve.promptFor(callOf("1", "run_command", { command: "ls" }), "ls", STEEL);
  assert.equal(approve.answerAt(picker.move(prompt, 1).index), "always");
  assert.equal(approve.answerAt(picker.move(prompt, -1).index), "no");
  assert.equal(approve.answerAt(picker.move(picker.move(prompt, 1), -1).index), "once");
});

test("a key that answers nothing neither approves nor refuses", () => {
  const prompt = approve.promptFor(callOf("1", "write_file", { path: "a.js" }), "a.js", STEEL);
  assert.equal(picker.byKey(prompt, "q"), undefined);
  assert.equal(approve.answerAt(picker.byKey(prompt, "y")), "once");
  assert.equal(approve.answerAt(picker.byKey(prompt, "2")), "always");
  assert.equal(picker.byKey(prompt, "9"), undefined);
});

test("answering always stops the question for the same file, not every file", async () => {
  const allowed = new Set<string>();
  const asked: string[] = [];
  const events = transcribe({
    emit: () => {},
    render: () => {},
    ask: (prompt, settle) => {
      asked.push(prompt.options.length === 3 ? "write_file" : "?");
      settle("always");
    },
    approved: (call) => allowed.has(approve.scopeFor(call).key),
    remember: (call) => allowed.add(approve.scopeFor(call).key),
    status: () => {},
    palette: STEEL,
  });

  const call = callOf("1", "write_file", { path: "a.js" });
  assert.equal(await events.approve(call), true);
  assert.equal(await events.approve(callOf("2", "edit_file", { path: "a.js" })), true);
  assert.equal(await events.approve(callOf("3", "write_file", { path: "b.js" })), true);
  assert.deepEqual(asked, ["write_file", "write_file"]);
});

test("denying a preview keeps the evidence on its rail", async () => {
  const blocks: Block[] = [];
  const events = transcribe({
    emit: (block) => blocks.push(block),
    render: () => {},
    ask: (_prompt, settle) => settle("no"),
    approved: () => false,
    remember: () => {},
    status: () => {},
    palette: STEEL,
  });
  const call = callOf("1", "edit_file", { path: "a.ts", old_text: "a", new_text: "b" });
  events.onToolCall(call);
  assert.equal(await events.approve(call), false);
  events.onToolResult(call, { kind: "tool_result", id: "1", output: "declined", isError: true }, "declined");

  const block = blocks[0];
  assert.equal(block?.kind === "tool" ? block.tone : undefined, "deny");
  assert.equal(block?.kind === "tool" ? block.right : undefined, "denied");
  assert.equal(block?.kind === "tool" ? block.body?.length : undefined, 2);
});

test("an interrupted approval reconciles a provisional denial on its rail", async () => {
  const blocks: Block[] = [];
  const events = transcribe({
    emit: (block) => blocks.push(block),
    render: () => {},
    ask: (_prompt, settle) => settle("no"),
    approved: () => false,
    remember: () => {},
    status: () => {},
    palette: STEEL,
  });
  const call = callOf("1", "write_file", { path: "a.ts", content: "value" });
  events.onToolCall(call);
  assert.equal(await events.approve(call), false);
  events.onToolResult(
    call,
    {
      kind: "tool_result",
      id: "1",
      output: "interrupted before completion",
      isError: true,
    },
    "interrupted",
  );

  const block = blocks[0];
  assert.equal(block?.kind === "tool" ? block.tone : undefined, "fail");
  assert.equal(block?.kind === "tool" ? block.right : undefined, "interrupted");
});
