import { test } from "node:test";
import assert from "node:assert/strict";
import { renderBatch } from "../src/batch-view.ts";
import { STEEL } from "../src/ui/theme.ts";
import type { Block } from "../src/tui/blocks.ts";

test("batch tool output keeps the content without full-screen padding", () => {
  const block: Block = {
    kind: "tool",
    name: "run_command",
    target: "npm test",
    right: "exit 0 · 1.2s",
    tone: "ok",
    body: [
      { kind: "out", text: "one" },
      { kind: "out", text: "two" },
    ],
  };

  const rows = renderBatch(block, 80, STEEL);
  assert.match(rows.join("\n"), /run_command\s+npm test/);
  assert.deepEqual(
    rows
      .filter((line) => /│\s+(?:one|two)$/.test(line))
      .map((line) => line.replace(/^.*│\s+/, "")),
    ["one", "two"],
  );
  assert.ok(rows.every((line) => !/[ \t]+$/.test(line)));
  assert.ok(rows.every((line, index) => line !== "" || rows[index - 1] !== ""));
  assert.notEqual(rows.at(-1), "");
});

test("informational transcript notices use the semantic gutter mark", () => {
  const rows = renderBatch(
    { kind: "notice", text: "context compacted", tone: "info" },
    80,
    STEEL,
  );

  assert.match(rows.join("\n"), /context compacted/);
  assert.match(rows.join("\n"), /·\s+context compacted/);
});

test("batch prose uses the supplied width without the interactive reading column", () => {
  const text = "word ".repeat(20).trimEnd();
  const rows = renderBatch({ kind: "answer", text }, 120, STEEL);
  assert.deepEqual(rows, ["", text]);
});

test("batch evidence is complete without interactive expansion hints", () => {
  const body = Array.from({ length: 20 }, (_, index) => ({ kind: "out" as const, text: `LINE_${index}` }));
  const block: Block = { kind: "tool", name: "run_command", target: "", right: "exit 1", tone: "fail", body };
  const rows = renderBatch(block, 80, STEEL);
  assert.deepEqual(rows.filter((line) => line.startsWith("│ ")).map((line) => line.slice(2)),
    body.map((detail) => detail.text));
  assert.equal(rows.filter((line) => line.includes("run_command")).length, 1);
  assert.doesNotMatch(rows.join("\n"), /ctrl\+o|earlier lines|other lines/);
});
