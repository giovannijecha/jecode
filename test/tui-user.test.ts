import { test } from "node:test";
import assert from "node:assert/strict";
import { render, renderAll } from "../src/tui/blocks.ts";
import { textWidth } from "../src/ui/width.ts";
import { STEEL } from "../src/ui/theme.ts";
import { strip } from "../dev/test-support/tui.ts";

test("user messages preserve literal Markdown, indentation, spaces, and line breaks", () => {
  const lines = [
    "# Request",
    "Keep **bold** and `src/**/*.ts` literal.",
    "[reference](https://example.test)",
    "* a literal bullet",
    "> a literal quote",
    "",
    "```ts",
    "  const value =  1;  ",
    "```",
    "",
  ];
  const block = { kind: "user", text: lines.join("\n") } as const;
  const width = 70;
  const drawn = strip(render(block, width, STEEL));

  assert.equal(drawn[0], "");
  assert.equal(drawn[1], " ".repeat(width));
  assert.deepEqual(drawn.slice(2, -1), lines.map((line) =>
    `  ${line}${" ".repeat(width - 2 - textWidth(line))}`
  ));
  assert.equal(drawn.at(-1), " ".repeat(width));
  assert.equal(block.text, lines.join("\n"));
});

test("literal user input stays distinct from rendered assistant Markdown", () => {
  const drawn = strip(renderAll([
    { kind: "user", text: "**keep this**\n`src/main.ts`" },
    { kind: "answer", text: "**Formatted answer** with `code`." },
  ], 60, STEEL)).join("\n");

  assert.match(drawn, /^  \*\*keep this\*\* +$/m);
  assert.match(drawn, /^  `src\/main\.ts` +$/m);
  assert.match(drawn, /^  Formatted answer with code\.$/m);
});

test("literal user wrapping preserves complete graphemes at narrow widths", () => {
  const text = "日本e\u0301👩‍💻🇮🇹".repeat(12);

  for (const width of [10, 38, 70]) {
    const drawn = strip(render({ kind: "user", text }, width, STEEL));
    const content = drawn.slice(2, -1).map((line) => line.slice(2).trimEnd());

    assert.ok(drawn.every((line) => textWidth(line) <= width));
    assert.ok(content.every((line) => line.isWellFormed()));
    assert.equal(content.join(""), text);
    assert.ok(content.every((line) => !line.startsWith("\u0301")));
    assert.ok(content.every((line) => !line.startsWith("\u200d") && !line.endsWith("\u200d")));
  }
});

test("literal user input neutralizes terminal controls before wrapping", () => {
  const escape = String.fromCharCode(27);
  const block = { kind: "user", text: `\t${escape}[2J\u202eend` } as const;
  const drawn = strip(render(block, 38, STEEL));

  assert.match(drawn[2] ?? "", /^    ␛\[2J\\u202eend +$/);
  assert.ok(drawn.every((line) => !line.includes(escape) && !line.includes("\u202e")));
  assert.ok(drawn.every((line) => textWidth(line) <= 38));
  assert.equal(block.text, `\t${escape}[2J\u202eend`);
});
