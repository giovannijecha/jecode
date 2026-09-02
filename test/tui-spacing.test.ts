import { test } from "node:test";
import assert from "node:assert/strict";
import type { Block } from "../src/tui/blocks.ts";
import { render, renderAll } from "../src/tui/blocks.ts";
import * as edit from "../src/tui/editor.ts";
import * as field from "../src/tui/field.ts";
import { compose } from "../src/tui/view.ts";
import { STEEL } from "../src/ui/theme.ts";

const ESC = String.fromCharCode(27);
const ANSI = new RegExp(ESC + "\\[[0-9;]*m", "g");

test("every conversation block owns one leading rhythm row", () => {
  const blocks: Block[] = [
    { kind: "user", text: "question" },
    { kind: "reasoning", text: "thinking" },
    { kind: "answer", text: "answer" },
    { kind: "tool", name: "read_file", target: "a.ts", right: "1 line", tone: "ok" },
    { kind: "notice", text: "ready", tone: "info" },
  ];

  for (const block of blocks) assert.equal(render(block, 50, STEEL)[0], "", block.kind);
});

test("a user turn separates its outer gap from its padded surface", () => {
  const width = 50;
  const drawn = plain(renderAll([
    { kind: "answer", text: "previous answer" },
    { kind: "user", text: "next question" },
  ], width, STEEL));
  const question = drawn.findIndex((line) => line.includes("next question"));

  assert.match(drawn[question - 3] ?? "", /previous answer/);
  assert.equal(drawn[question - 2], "");
  assert.equal(drawn[question - 1], " ".repeat(width));
});

test("consecutive tool calls join one execution rail without repeated gaps", () => {
  const drawn = plain(renderAll([
    { kind: "tool", name: "read_file", target: "a.ts", right: "1 line", tone: "ok" },
    { kind: "tool", name: "search_text", target: "needle", right: "2 matches", tone: "ok" },
  ], 60, STEEL));
  const first = drawn.findIndex((line) => line.includes("read_file"));
  const second = drawn.findIndex((line) => line.includes("search_text"));
  assert.equal(first, 1);
  assert.equal(second, first + 1);
  assert.equal(drawn.filter((line) => line === "").length, 1);
});

test("a short transcript grows upward from the composer", () => {
  const frame = compose(
    {
      blocks: [{ kind: "answer", text: "latest answer" }],
      editor: edit.EMPTY,
      scroll: 0,
      pal: STEEL,
      footer: { workspace: "~/work", model: "model", effort: "high" },
    },
    { rows: 24, cols: 80 },
  );
  const rows = plain(frame.rows);
  const latest = rows.findIndex((line) => line.includes("latest answer"));
  const topRule = (frame.cursor?.row ?? 0) - 1;
  assert.equal(latest, 18);
  assert.deepEqual(rows.slice(latest + 1, topRule), [""]);
  assert.ok(rows.slice(0, latest - 1).every((line) => line === ""));
});

test("a writable field carries the same active arrow as a picker", () => {
  const shown = plain(field.panel({ title: [], editor: edit.of("64000"), secret: false }, 60, STEEL));
  assert.equal(shown[1], "→ 64000");
});

function plain(rows: readonly string[]): string[] {
  return rows.map((row) => row.replace(ANSI, ""));
}
