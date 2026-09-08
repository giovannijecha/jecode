import { test } from "node:test";
import assert from "node:assert/strict";
import { textWidth } from "../src/ui/width.ts";

test("printable ASCII width avoids allocating grapheme records", t => {
  const segment = t.mock.method(Intl.Segmenter.prototype, "segment");
  const printable = Array.from({ length: 95 }, (_, index) => String.fromCharCode(index + 32)).join("");
  assert.equal(textWidth(""), 0);
  assert.equal(textWidth(printable), 95);
  assert.equal(textWidth(printable.repeat(1_000)), 95_000);
  assert.equal(segment.mock.callCount(), 0);
});

test("ASCII controls retain their zero-cell width at either edge and inside text", () => {
  for (const code of [...Array.from({ length: 32 }, (_, index) => index), 127]) {
    const control = String.fromCharCode(code);
    for (const input of [control + "abc", "a" + control + "bc", "abc" + control]) {
      assert.equal(textWidth(input), 3, `control ${code}`);
    }
  }
  assert.equal(textWidth("a\r\nb"), 2);
});

test("mixed text retains combining, emoji, fullwidth and malformed-surrogate widths", () => {
  const cases: Array<[string, number]> = [
    ["e\u0301", 1], ["\u0301", 0], ["\u00e8", 1], ["\u65e5", 2],
    ["\uff21", 2], ["1\ufe0f\u20e3", 2], ["\u2764\ufe0e", 1],
    ["\u2764\ufe0f", 2], ["\u200d", 0], ["\ud800", 1],
    [String.fromCodePoint(0x1f469, 0x200d, 0x1f4bb), 2],
    [String.fromCodePoint(0x1f1ee, 0x1f1f9), 2],
    [String.fromCodePoint(0x20000), 2],
  ];
  for (const [text, width] of cases) {
    assert.equal(textWidth(text), width);
    assert.equal(textWidth(`left ${text} right`), width + 11);
  }
});
