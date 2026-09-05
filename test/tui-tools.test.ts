import { test } from "node:test";
import assert from "node:assert/strict";
import type { View } from "../src/tui/view.ts";
import { compose } from "../src/tui/view.ts";
import { textWidth } from "../src/ui/width.ts";
import type { Block } from "../src/tui/blocks.ts";
import { renderAll } from "../src/tui/blocks.ts";
import { STEEL } from "../src/ui/theme.ts";
import { base, stage, callOf, strip } from "../dev/test-support/tui.ts";

test("a pending tool keeps one stable state node and its elapsed time", () => {
  const view: View = {
    ...base(),
    blocks: [{
      kind: "tool",
      name: "run_command",
      target: "npm test",
      right: "running",
      tone: "pending",
      startedAt: 0,
    }],
    status: "Running run_command",
  };
  const first = strip(compose({ ...view, now: 1_000 }, { rows: 24, cols: 80 }).rows).join("\n");
  const later = strip(compose({ ...view, now: 2_000 }, { rows: 24, cols: 80 }).rows).join("\n");
  assert.match(first, /^┌ npm test$/m);
  assert.match(later, /^┌ npm test$/m);
  assert.match(first, /run_command\s+○ running/);
  assert.match(later, /run_command\s+○ running/);
  assert.match(first, /running · 1\.0s/);
  assert.match(later, /running · 2\.0s/);
  assert.match(later, /esc to interrupt/);
});

test("an edit is diffed from the call, so approval is about the change", () => {
  const { blocks, events } = stage([]);
  events.onToolCall(
    callOf("1", "edit_file", { path: "a.js", old_text: "let x = 1;", new_text: "const x = 1;" }),
  );

  const [block] = blocks;
  assert.equal(block?.kind, "tool");
  assert.deepEqual(
    block?.kind === "tool" ? block.body?.map((d) => `${d.kind}:${d.text}`) : undefined,
    ["del:let x = 1;", "add:const x = 1;"],
  );
  if (block?.kind !== "tool") return;
  assert.deepEqual(block.body?.[0], {
    kind: "del",
    text: "let x = 1;",
    oldLine: 1,
    emphasis: { start: 0, length: 3 },
  });
  assert.deepEqual(block.body?.[1], {
    kind: "add",
    text: "const x = 1;",
    newLine: 1,
    emphasis: { start: 0, length: 5 },
  });
  const rendered = strip(renderAll([block], 60, STEEL)).join("\n");
  assert.match(rendered, /-\s+1 let x = 1;/);
  assert.match(rendered, /\+\s+1 const x = 1;/);
});

test("an emoji replacement emphasizes complete graphemes", () => {
  const { blocks, events } = stage([]);
  const before = String.fromCodePoint(0x1f600);
  const after = String.fromCodePoint(0x1f601);
  events.onToolCall(
    callOf("emoji", "edit_file", { path: "emoji.txt", old_text: before, new_text: after }),
  );

  const [block] = blocks;
  assert.equal(block?.kind, "tool");
  if (block?.kind !== "tool") return;
  const removed = block.body?.[0];
  const added = block.body?.[1];
  assert.equal(removed?.kind, "del");
  assert.equal(added?.kind, "add");
  if (removed?.kind !== "del" || added?.kind !== "add") return;
  assert.deepEqual(removed.emphasis, { start: 0, length: before.length });
  assert.deepEqual(added.emphasis, { start: 0, length: after.length });
});

test("a tool is a compact execution rail with evidence beneath it", () => {
  const block: Block = {
    kind: "tool",
    name: "run_command",
    target: "ls",
    right: "exit 0",
    tone: "ok",
    body: [
      { kind: "out", text: "one" },
      { kind: "out", text: "two" },
    ],
  };

  const ESCAPE = String.fromCharCode(27);
  const bare = renderAll([block], 60, STEEL).map((r) => r.replace(new RegExp(`${ESCAPE}\[[0-9;]*m`, "g"), ""));
  assert.match(bare.join("\n"), /^┌ ls$/m);
  assert.match(bare.join("\n"), /│\s+run_command\s+✓ exit 0/);
  assert.match(bare.join("\n"), /│\s+one/);
  assert.match(bare.join("\n"), /│\s+two/);
  assert.equal(bare[0], "");
  assert.ok(bare.every((line) => textWidth(line) <= 60));
  assert.ok(bare.slice(1).every((line) => !/[ \t]+$/.test(line)));
});

test("a write over an existing file is shown as the change, not as a new file", () => {
  const { blocks, events } = stage([]);
  events.onToolCall(callOf("1", "write_file", { path: "conf.json", content: "a\nCHANGED\nc\n" }), {
    before: "a\nb\nc\n",
    after: "a\nCHANGED\nc\n",
  });

  const [block] = blocks;
  assert.deepEqual(
    block?.kind === "tool" ? block.body?.map((d) => `${d.kind}:${d.text}`) : undefined,
    ["keep:a", "del:b", "add:CHANGED", "keep:c"],
  );
});

test("a compact edit shows only changed lines while expansion restores context", () => {
  const { blocks, events } = stage([]);
  events.onToolCall(callOf("1", "edit_file", {
    path: "config.ts",
    old_text: "oldValue",
    new_text: "newValue",
  }), {
    before: "one\ntwo\nthree\nfour\nfive\nsix\noldValue\nseven\neight\nnine\nten\n",
    after: "one\ntwo\nthree\nfour\nfive\nsix\nnewValue\nseven\neight\nnine\nten\n",
  });

  const block = blocks[0];
  assert.equal(block?.kind, "tool");
  if (block?.kind !== "tool") return;

  const compact = strip(renderAll([block], 60, STEEL)).join("\n");
  assert.match(compact, /-\s+7 oldValue/);
  assert.match(compact, /\+\s+7 newValue/);
  assert.doesNotMatch(compact, /five|six|seven|eight|unchanged|lines hidden/);

  block.expanded = true;
  const expanded = strip(renderAll([block], 60, STEEL)).join("\n");
  assert.match(expanded, /five|six/);
  assert.match(expanded, /seven|eight/);
  assert.match(expanded, /unchanged/);
});

test("a local edit in a large file keeps a bounded semantic preview", () => {
  const { blocks, events } = stage([]);
  const original = Array.from({ length: 3_600 }, (_, index) => `value-${index + 1}`);
  const changed = [...original];
  changed[3_566] = "VALUE-3567";
  events.onToolCall(callOf("1", "edit_file", {
    path: "styles.css",
    old_text: "value-3567",
    new_text: "VALUE-3567",
  }), {
    before: original.join("\n"),
    after: changed.join("\n"),
  });

  const block = blocks[0];
  assert.equal(block?.kind, "tool");
  if (block?.kind !== "tool") return;
  assert.equal(block.body?.filter((detail) => detail.kind === "del").length, 1);
  assert.equal(block.body?.filter((detail) => detail.kind === "add").length, 1);
  assert.ok((block.body?.length ?? 0) < 10);

  const compact = strip(renderAll([block], 60, STEEL)).join("\n");
  assert.match(compact, /-3567 value-3567/);
  assert.match(compact, /\+3567 VALUE-3567/);
  assert.doesNotMatch(compact, /more changed lines/);
});

test("large file diffs share one bounded preview while expansion keeps every change", () => {
  const { blocks, events } = stage([]);
  const content = Array.from({ length: 22 }, (_, index) => `value-${index + 1}`).join("\n");
  events.onToolCall(callOf("1", "write_file", { path: "large.txt", content }));

  const block = blocks[0];
  assert.equal(block?.kind, "tool");
  if (block?.kind !== "tool") return;
  assert.equal(block.body?.filter((detail) => detail.kind === "add").length, 22);

  const compact = strip(renderAll([block], 60, STEEL)).join("\n");
  assert.match(compact, /value-3/);
  assert.doesNotMatch(compact, /value-4(?:\D|$)/);
  assert.match(compact, /16 more changed lines/);
  assert.match(compact, /ctrl\+o/);
  assert.match(compact, /value-20/);
  assert.match(compact, /value-22/);

  block.expanded = true;
  const expanded = strip(renderAll([block], 60, STEEL)).join("\n");
  assert.match(expanded, /value-9/);
  assert.doesNotMatch(expanded, /more changed lines/);
});

test("tool output is retained in full and only collapsed while rendering", () => {
  const { blocks, events } = stage([]);
  const call = callOf("1", "run_command", { command: "many" });
  events.onToolCall(call);
  events.onToolResult(
    call,
    { kind: "tool_result", id: "1", output: Array.from({ length: 20 }, (_, index) => `line ${index}`).join("\n"), isError: false },
    "exit 0",
  );
  const block = blocks[0];
  assert.equal(block?.kind, "tool");
  if (block?.kind !== "tool") return;
  assert.equal(block.body?.length, 20);
  const compact = strip(renderAll([block], 60, STEEL)).join("\n");
  assert.match(compact, /16 earlier lines · ctrl\+o/);
  assert.deepEqual(compact.match(/line \d+/g), ["line 16", "line 17", "line 18", "line 19"]);
  block.expanded = true;
  const expanded = strip(renderAll([block], 60, STEEL)).join("\n");
  assert.match(expanded, /line 19/);
  assert.doesNotMatch(expanded, /ctrl\+o expand/);
});

test("conversation blocks separate user surface, free reasoning, and tool rail", () => {
  const blocks: Block[] = [
    { kind: "user", text: "ask" },
    { kind: "reasoning", text: "think", expanded: true },
    { kind: "tool", name: "read_file", target: "a.ts", right: "1 line", tone: "ok" },
    { kind: "answer", text: "done" },
  ];
  const drawn = strip(renderAll(blocks, 50, STEEL)).join("\n");
  assert.match(drawn, /^  ask\s*$/m);
  assert.match(drawn, /^think$/m);
  assert.match(drawn, /read_file\s+✓ 1 line/);
  assert.match(drawn, /^done$/m);
  assert.match(drawn, /think\n┌ a\.ts/);
  assert.match(drawn, /└─ read_file[^\n]+\n\ndone/);
});

test("tool identity is semantic rather than encoded in decorative glyphs", () => {
  const { blocks, events } = stage([]);
  const names = [
    "read_file",
    "list_dir",
    "find_files",
    "search_text",
    "edit_file",
    "write_file",
    "run_command",
    "invented",
  ];

  for (const name of names) events.onToolCall(callOf(name, name, {}));

  const tools = blocks.filter((block) => block.kind === "tool");
  assert.deepEqual(tools.map((block) => block.name), names);
  assert.ok(tools.every((block) => !("mark" in block)));
});
