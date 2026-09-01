import { test } from "node:test";
import assert from "node:assert/strict";
import { configureColor, fitSegs, paint, plainLen } from "../src/ui/render.ts";
import { terminalText } from "../src/ui/terminal-text.ts";
import { textWidth } from "../src/ui/width.ts";

const ESC = String.fromCharCode(27);
const BEL = String.fromCharCode(7);
const CSI = String.fromCharCode(0x9b);
const ESC_PICTURE = String.fromCodePoint(0x241b);
const LF_PICTURE = String.fromCodePoint(0x240a);
const ELLIPSIS = String.fromCodePoint(0x2026);

test("neutralizes terminal escapes and every active control character", () => {
  const unsafe = [
    `before${ESC}]8;;https://example.test${BEL}link${ESC}]8;;${BEL}`,
    `${ESC}]52;c;Y2xpcGJvYXJk${BEL}`,
    `${ESC}[31mred${ESC}[0m`,
    `${CSI}2J`,
    String.fromCharCode(0, 8, 13, 127),
  ].join("");

  const safe = terminalText(unsafe);

  assert.doesNotMatch(safe, /[\u0000-\u001f\u007f-\u009f]/u);
  assert.ok(safe.includes(`${ESC_PICTURE}]52`));
  assert.match(safe, /\\u009b2J/);
});

test("keeps only structural newlines and expands tabs deterministically", () => {
  assert.equal(terminalText("a\tb\nc", { multiline: true }), "a  b\nc");
  assert.equal(terminalText("a\nb"), `a${LF_PICTURE}b`);
});

test("layout measures the same neutralized text that paint emits", () => {
  configureColor(false);
  const seg = { text: `a${ESC}[2Jb` };
  const shown = paint(seg);

  assert.equal(plainLen([seg]), textWidth(shown));
  assert.equal(
    fitSegs([seg], 4).map((part) => part.text).join(""),
    `a${ESC_PICTURE}${ELLIPSIS}b`,
  );
});

test("optional context disappears instead of rendering as a fragment", () => {
  const state = { text: "Writing · 2s" };
  const hint = { text: " · esc to interrupt", optional: true };

  assert.deepEqual(fitSegs([state, hint], plainLen([state])), [state]);
});
