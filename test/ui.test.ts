import { test } from "node:test";
import assert from "node:assert/strict";
import { charWidth, elide, textWidth, wrapText } from "../src/ui/width.ts";
import { markdown } from "../src/ui/markdown.ts";
import { highlight } from "../src/ui/highlight.ts";
import { STEEL } from "../src/ui/theme.ts";
import { condense, diff } from "../src/ui/diff.ts";

test("a glyph the terminal draws wide is measured as two cells", () => {
  assert.equal(charWidth("a"), 1);
  assert.equal(charWidth("日"), 2);
  assert.equal(charWidth("🙂"), 2);
});

test("default emoji and regional flags occupy two terminal cells", () => {
  for (const code of [0x231a, 0x26a1, 0x2705, 0x274c, 0x2b50]) {
    assert.equal(charWidth(String.fromCodePoint(code)), 2);
  }
  assert.equal(charWidth(String.fromCodePoint(0x1f1ee, 0x1f1f9)), 2);
  assert.equal(charWidth(String.fromCodePoint(0x231a, 0xfe0e)), 1);
});

test("a combining accent takes no cell of its own", () => {
  const composed = `e${String.fromCodePoint(0x0301)}`;
  assert.equal(textWidth(composed), 1);
  assert.equal(textWidth("città"), 5);
});

test("a value too long keeps both of its ends", () => {
  const path = "src/providers/anthropic-stream.ts";
  const cut = elide(path, 20);
  assert.equal(textWidth(cut), 20);
  assert.ok(cut.startsWith("src/"));
  assert.ok(cut.endsWith(".ts"));
  assert.ok(cut.includes("…"));
});

test("a word wider than the row is broken rather than lost", () => {
  const rows = wrapText("a/very/long/path/that/never/fits", 10);
  assert.ok(rows.every((line) => textWidth(line) <= 10));
  assert.equal(rows.join(""), "a/very/long/path/that/never/fits");
});

test("wrapping measures cells, not characters", () => {
  const rows = wrapText("日本語 のテキスト です", 8);
  assert.ok(rows.every((line) => textWidth(line) <= 8));
});

test("markdown spends the notation instead of printing it", () => {
  const rows = markdown("una **cosa** con `codice`", 60, STEEL);
  const text = rows.flatMap((r) => r.segs.map((s) => s.text)).join("");
  assert.ok(!text.includes("**"));
  assert.ok(!text.includes("`"));
  assert.ok(text.includes("cosa"));
  assert.ok(text.includes("codice"));
  const code = rows.flatMap((r) => r.segs).find((seg) => seg.text === "codice");
  assert.equal(code?.bg, undefined);
  assert.deepEqual(code?.fg, STEEL.technical);
});

test("technical text is vivid while structural and secondary text stay distinct", () => {
  const [list] = markdown("- read `src/main.ts` and [docs](https://example.test)", 60, STEEL);
  const segments = list?.segs ?? [];
  assert.deepEqual(segments.find((segment) => segment.text === "- ")?.fg, STEEL.technical);
  assert.deepEqual(segments.find((segment) => segment.text === "src/main.ts")?.fg, STEEL.technical);
  assert.deepEqual(segments.find((segment) => segment.text === "docs")?.fg, STEEL.technical);
  assert.notDeepEqual(STEEL.technical, STEEL.accent);
  assert.notDeepEqual(STEEL.ink.muted, STEEL.ink.dim);
});

test("a paragraph is its lines joined, not one row each", () => {
  const rows = markdown("prima riga\nseconda riga", 60, STEEL);
  assert.equal(rows.length, 1);
});

test("a fenced block uses muted delimiters without a legacy surface", () => {
  const rows = markdown("prosa\n\n```ts\nconst a = 1;\n```", 60, STEEL);
  assert.ok(rows.flatMap((rendered) => rendered.segs).every((seg) => seg.bg === undefined));
  assert.deepEqual(
    rows.slice(2).map((rendered) => rendered.segs.map((seg) => seg.text).join("")),
    ["```ts", "  const a = 1;", "```"],
  );
});

test("a list item hangs its wrapped rows under the text, not the bullet", () => {
  const rows = markdown("- una voce che deve per forza andare a capo qui", 24, STEEL);
  assert.ok(rows.length > 1);
  assert.ok((rows[0]?.segs ?? []).some((s) => s.text.includes("-")));
  assert.ok(!(rows[1]?.segs ?? []).some((s) => s.text.includes("-")));
});

test("highlighting names the parts a reader looks for", () => {
  const [line] = highlight(["const n = 0x1f; // nota"], "ts");
  const roles = (line ?? []).map((token) => `${token.role}:${token.text.trim()}`);
  assert.ok(roles.includes("keyword:const"));
  assert.ok(roles.includes("number:0x1f"));
  assert.ok(roles.includes("comment:// nota"));
});

test("a block comment stays a comment on the rows after it opens", () => {
  const rows = highlight(["/* apre", "ancora", "chiude */ const x = 1;"], "ts");
  assert.equal(rows[1]?.[0]?.role, "comment");
  assert.ok((rows[2] ?? []).some((token) => token.role === "keyword"));
});

test("a language nobody knows gets no colour rather than the wrong one", () => {
  const [line] = highlight(["const n = 1;"], "brainfuck");
  assert.deepEqual(line, [{ text: "const n = 1;", role: "plain" }]);
});

test("a diff keeps what did not move and marks what did", () => {
  const rows = diff("a\nb\nc\n", "a\nB\nc\n");
  assert.deepEqual(
    rows.map((r) => `${r.kind}:${r.text}`),
    ["keep:a", "del:b", "add:B", "keep:c"],
  );
});

test("a trailing newline is the file ending, not a line of it", () => {
  assert.deepEqual(diff("a\n", "a\nb\n").map((r) => r.kind), ["keep", "add"]);
});

test("an empty side of a diff has no invented blank line", () => {
  assert.deepEqual(diff("", "a\n").map((row) => `${row.kind}:${row.text}`), ["add:a"]);
  assert.deepEqual(diff("a\n", "").map((row) => `${row.kind}:${row.text}`), ["del:a"]);
  assert.deepEqual(diff("", ""), []);
});

test("nothing changed means nothing is marked", () => {
  assert.ok(diff("x\ny\n", "x\ny\n").every((r) => r.kind === "keep"));
});

test("condensing keeps the change in sight and counts what it dropped", () => {
  const before = Array.from({ length: 12 }, (_, i) => `line ${i}`).join("\n");
  const after = before.replace("line 6", "LINE 6");
  const shown = condense(diff(before, after), 1);

  assert.deepEqual(
    shown.map((r) => (r.kind === "gap" ? `gap:${r.skipped}` : `${r.kind}:${r.text}`)),
    ["gap:5", "keep:line 5", "del:line 6", "add:LINE 6", "keep:line 7", "gap:4"],
  );
});

const flat = (rows: { segs: { text: string }[] }[]): string[] =>
  rows.map((row) => row.segs.map((seg) => seg.text).join(""));

test("a table is drawn as columns, not printed as pipes", () => {
  const rows = flat(
    markdown(
      ["| Provider | Model |", "| --- | --- |", "| anthropic | sonnet |", "| ollama | local |"].join("\n"),
      60,
      STEEL,
    ),
  );

  assert.deepEqual(rows, [
    "Provider   Model ",
    "─────────  ──────",
    "anthropic  sonnet",
    "ollama     local ",
  ]);
});

test("a column is as wide as its widest cell, measured in cells", () => {
  const rows = flat(markdown(["| a |", "| --- |", "| 日本 |"].join("\n"), 60, STEEL));
  // Four cells for the two wide glyphs, so the header pads to match.
  assert.deepEqual(rows, ["a   ", "────", "日本"]);
});

test("an alignment marker moves the cell inside its column", () => {
  const rows = flat(markdown(["| n |", "| ---: |", "| 7 |"].join("\n"), 60, STEEL));
  assert.deepEqual(rows, ["n", "─", "7"]);

  const wide = flat(markdown(["| total |", "| ---: |", "| 7 |"].join("\n"), 60, STEEL));
  assert.equal(wide[2], "    7");
});

test("a header row alone is prose until its delimiter arrives", () => {
  // Text streams in, so a half-written table must not be a parse error.
  const rows = flat(markdown("| Provider | Model |", 60, STEEL));
  assert.deepEqual(rows, ["| Provider | Model |"]);
});

test("a fenced block carries muted fences rather than the legacy rail", () => {
  const rows = flat(markdown("```js\nlet x = 1;\n```", 60, STEEL));
  assert.deepEqual(rows, ["```js", "  let x = 1;", "```"]);
  assert.ok(rows.every((row) => !row.includes("▌")));
});

test("the renderer receives one complete dark Steel identity", () => {
  assert.deepEqual(STEEL.accent, [102, 155, 210]);
  assert.deepEqual(STEEL.technical, [78, 201, 232]);
  assert.deepEqual(STEEL.rule, [53, 80, 110]);
  assert.deepEqual(STEEL.ink.fg, [220, 224, 229]);
  assert.deepEqual(STEEL.ink.muted, [156, 169, 183]);
  assert.deepEqual(STEEL.ink.dim, [112, 124, 137]);
  assert.deepEqual(STEEL.ink.added, [134, 203, 146]);
  assert.deepEqual(STEEL.ink.removed, [232, 112, 112]);
  assert.equal("inset" in STEEL.surface, false);
});

test("prose can use a readable measure while code keeps the full row", () => {
  const words = Array.from({ length: 50 }, () => "word").join(" ");
  const prose = markdown(words, 150, STEEL, 40);
  assert.ok(prose.length > 1);
  const codeRows = flat(markdown(`\`\`\`txt\n${"x".repeat(100)}\n\`\`\``, 150, STEEL, 40));
  assert.equal(codeRows[1]?.length, 102); // two-cell indent and all 100 code cells
});
