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
  assert.match(rows.join("\n"), /run_command npm test/);
  assert.deepEqual(
    rows.filter((line) => line.trim() === "one" || line.trim() === "two").map((line) => line.trim()),
    ["one", "two"],
  );
  assert.ok(rows.every((line) => !/[ \t]+$/.test(line)));
  assert.ok(rows.every((line, index) => line !== "" || rows[index - 1] !== ""));
  assert.notEqual(rows.at(-1), "");
});
