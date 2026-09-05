import { test } from "node:test";
import assert from "node:assert/strict";
import type { ToolBlock } from "../src/transcript-types.ts";
import { STEEL } from "../src/ui/theme.ts";
import { textWidth } from "../src/ui/width.ts";

const ESC = String.fromCharCode(27);
const ANSI = new RegExp(`${ESC}\\[[0-9;]*m`, "g");

test("historical diff emphasis cannot split an emoji with or without color", async () => {
  const descriptor = Object.getOwnPropertyDescriptor(process.stdout, "isTTY");
  const noColor = process.env["NO_COLOR"];
  const term = process.env["TERM"];

  try {
    Object.defineProperty(process.stdout, "isTTY", { configurable: true, value: true });
    delete process.env["NO_COLOR"];
    process.env["TERM"] = "xterm-256color";

    const { renderAll } = await import("../src/tui/blocks.ts");
    const { configureColor } = await import("../src/ui/render.ts");
    const emoji = String.fromCodePoint(0x1f600);
    const block: ToolBlock = {
      kind: "tool",
      name: "edit_file",
      target: "emoji.txt",
      right: "1 replacement",
      tone: "ok",
      body: [{
        kind: "del",
        text: emoji,
        oldLine: 1,
        emphasis: { start: 1, length: 1 },
      }],
    };
    for (const color of [true, false]) {
      configureColor(color);
      const rows = renderAll([block], 60, STEEL);
      const encoded = rows.map((row) => Buffer.from(row, "utf8").toString("utf8"));

      assert.ok(rows.every((row) => row.isWellFormed()));
      assert.ok(encoded.every((row) => !row.includes(String.fromCodePoint(0xfffd))));
      assert.match(rows.map((row) => row.replace(ANSI, "")).join("\n"), new RegExp(emoji, "u"));
      assert.equal(rows.some((row) => row.includes(ESC)), color);
    }
  } finally {
    if (descriptor === undefined) delete (process.stdout as { isTTY?: boolean }).isTTY;
    else Object.defineProperty(process.stdout, "isTTY", descriptor);
    if (noColor === undefined) delete process.env["NO_COLOR"];
    else process.env["NO_COLOR"] = noColor;
    if (term === undefined) delete process.env["TERM"];
    else process.env["TERM"] = term;
  }
});

test("settled tool markers retain semantic success and failure colours", async () => {
  const { renderAll } = await import("../src/tui/blocks.ts");
  const { configureColor } = await import("../src/ui/render.ts");
  const colour = (rgb: readonly number[], mark: string) => `${ESC}[38;2;${rgb.join(";")}m${mark}`;

  try {
    configureColor(true);
    const success = renderAll([
      { kind: "tool", name: "read_file", target: "a.ts", right: "1 line", tone: "ok" },
    ], 60, STEEL).join("\n");
    const failure = renderAll([
      { kind: "tool", name: "run_command", target: "npm test", right: "exit 1", tone: "fail" },
    ], 60, STEEL).join("\n");

    assert.ok(success.includes(colour(STEEL.ink.added, "✓")));
    assert.ok(failure.includes(colour(STEEL.ink.removed, "×")));
  } finally {
    configureColor(false);
  }
});

test("user turns use half-cell Slate edges with the same spacing in monochrome", async () => {
  const descriptor = Object.getOwnPropertyDescriptor(process.stdout, "isTTY");
  const noColor = process.env["NO_COLOR"];
  const term = process.env["TERM"];

  try {
    Object.defineProperty(process.stdout, "isTTY", { configurable: true, value: true });
    delete process.env["NO_COLOR"];
    process.env["TERM"] = "xterm-256color";

    const { renderAll } = await import("../src/tui/blocks.ts");
    const { configureColor } = await import("../src/ui/render.ts");
    const surface = `${ESC}[48;2;${STEEL.surface.subtle.join(";")}m`;
    const edge = `${ESC}[38;2;${STEEL.surface.subtle.join(";")}m`;
    try {
      for (const width of [38, 70]) {
        configureColor(true);
        const coloured = renderAll([{ kind: "user", text: "distinct turn" }], width, STEEL);
        const plain = coloured.map((line) => line.replace(ANSI, ""));
        assert.equal(coloured.length, 4);
        assert.equal(plain[0], "");
        assert.equal(plain[1], "▄".repeat(width));
        assert.equal(plain[3], "▀".repeat(width));
        assert.ok(coloured[1]?.includes(edge));
        assert.ok(coloured[3]?.includes(edge));
        assert.ok(!coloured[1]?.includes(surface));
        assert.ok(!coloured[3]?.includes(surface));
        assert.ok(coloured[2]?.includes(surface));
        assert.match(plain[2] ?? "", /^  distinct turn\s*$/);
        assert.ok(plain.every((line) => textWidth(line) <= width));

        configureColor(false);
        const monochrome = renderAll([{ kind: "user", text: "distinct turn" }], width, STEEL);
        assert.equal(monochrome.length, coloured.length);
        assert.ok(monochrome.every((line) => !line.includes(ESC)));
        assert.equal(monochrome[1], " ".repeat(width));
        assert.equal(monochrome[3], " ".repeat(width));
        assert.equal(monochrome[2], plain[2]);
      }
    } finally {
      configureColor(false);
    }
  } finally {
    if (descriptor === undefined) delete (process.stdout as { isTTY?: boolean }).isTTY;
    else Object.defineProperty(process.stdout, "isTTY", descriptor);
    if (noColor === undefined) delete process.env["NO_COLOR"];
    else process.env["NO_COLOR"] = noColor;
    if (term === undefined) delete process.env["TERM"];
    else process.env["TERM"] = term;
  }
});
