import { test } from "node:test";
import assert from "node:assert/strict";
import type { Block } from "../src/tui/blocks.ts";
import { render, renderAll } from "../src/tui/blocks.ts";
import * as edit from "../src/tui/editor.ts";
import * as field from "../src/tui/field.ts";
import { compose } from "../src/tui/view.ts";
import { STEEL } from "../src/ui/theme.ts";
import { textWidth } from "../src/ui/width.ts";

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

test("a user turn keeps one outer gap and a padded semantic surface", () => {
  const width = 50;
  const drawn = plain(renderAll([
    { kind: "answer", text: "previous answer" },
    { kind: "user", text: "next question" },
  ], width, STEEL));
  const question = drawn.findIndex((line) => line.includes("next question"));

  assert.match(drawn[question - 3] ?? "", /previous answer/);
  assert.equal(drawn[question - 2], "");
  assert.equal(drawn[question - 1], " ".repeat(width));
  assert.match(drawn[question] ?? "", /^  next question\s+$/);
  assert.equal(drawn[question + 1], " ".repeat(width));
});

test("a wrapped user surface stays inside narrow terminal bounds", () => {
  for (const width of [38, 50]) {
    const drawn = render({
      kind: "user",
      text: "A deliberately long user message that wraps across several rows without leaving its surface.",
    }, width, STEEL);

    assert.ok(drawn.every((line) => textWidth(line) <= width));
    assert.ok(drawn.length > 4);
    assert.match(plain(drawn).join("\n"), /^  A deliberately long/m);
  }
});

test("assistant prose and tool records share a two-cell inset and an 86-cell measure", () => {
  const blocks: Block[] = [
    { kind: "answer", text: "bounded answer ".repeat(30) },
    { kind: "reasoning", text: "bounded reasoning ".repeat(30), expanded: true },
    { kind: "tool", name: "read_file", target: "folder/".repeat(30), right: "1 line", tone: "ok" },
  ];
  for (const width of [38, 100, 200]) {
    const drawn = plain(renderAll(blocks, width, STEEL)).filter((line) => line !== "");
    assert.ok(drawn.every((line) => line.startsWith("  ")));
    assert.ok(drawn.every((line) => textWidth(line) <= Math.min(width - 2, 88)));
    assert.ok(drawn.some((line) => textWidth(line) > Math.min(width - 10, 75)));
  }
});

test("consecutive tool calls keep one gap between their independent records", () => {
  const drawn = plain(renderAll([
    { kind: "tool", name: "read_file", target: "a.ts", right: "1 line", tone: "ok" },
    { kind: "tool", name: "search_text", target: "needle", right: "2 matches", tone: "ok" },
  ], 60, STEEL));
  const first = drawn.findIndex((line) => line === "  ┌ a.ts");
  const second = drawn.findIndex((line) => line === "  ┌ needle");
  assert.equal(first, 1);
  assert.equal(second, first + 3);
  assert.equal(drawn[second - 1], "");
  assert.equal(drawn.filter((line) => line === "").length, 2);
});

test("reasoning leads directly into a tool while the following thought keeps one gap", () => {
  const drawn = plain(renderAll([
    { kind: "reasoning", text: "inspect first" },
    { kind: "tool", name: "read_file", target: "a.ts", right: "1 line", tone: "ok" },
    { kind: "reasoning", text: "continue after the result" },
  ], 60, STEEL));
  const tool = drawn.findIndex((line) => line === "  ┌ a.ts");
  const continued = drawn.findIndex((line) => line.includes("continue after"));

  assert.equal(drawn[tool - 1], "  inspect first");
  assert.equal(drawn[continued - 1], "");
  assert.equal(drawn[continued], "  continue after the result");
});

test("response groups keep one leading gap except after a continuing thought", () => {
  const tool: Block = { kind: "tool", name: "read_file", target: "a.ts", right: "4 lines", tone: "ok" };
  const thought: Block = { kind: "reasoning", text: "Inspect the boundary.", live: false };
  const answer: Block = { kind: "answer", text: "The boundary is intact." };
  const cases: readonly (readonly [Block, Block, number])[] = [
    [tool, tool, 1], [tool, thought, 1], [tool, answer, 1], [thought, tool, 0], [thought, thought, 0],
  ];
  for (const [previous, next, gap] of cases) {
    const rows = plain(render(next, 100, STEEL, { previous, reducedMotion: true }));
    assert.equal(rows.findIndex((line) => line.trim() !== ""), gap, `${previous.kind} → ${next.kind}`);
  }
});

test("a short transcript grows upward from the composer", () => {
  const frame = compose(
    {
      blocks: [{ kind: "answer", text: "latest answer" }],
      editor: edit.EMPTY,
      scroll: 0,
      pal: STEEL,
      footer: { workspace: "~/work", provider: "Anthropic", model: "model", effort: "high" },
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

test("a writable field keeps its active input prompt", () => {
  const shown = plain(field.panel({ title: [], editor: edit.of("64000"), secret: false }, 60, STEEL));
  assert.equal(shown[1], "→ 64000");
});

function plain(rows: readonly string[]): string[] {
  return rows.map((row) => row.replace(ANSI, ""));
}
