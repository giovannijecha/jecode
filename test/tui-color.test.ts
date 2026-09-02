import { test } from "node:test";
import assert from "node:assert/strict";
import type { ToolBlock } from "../src/transcript-types.ts";
import { STEEL } from "../src/ui/theme.ts";

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
