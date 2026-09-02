import { test } from "node:test";
import assert from "node:assert/strict";
import { leadingText, trailingText } from "../src/tools/text-boundary.ts";

const clusters = [
  ["emoji", String.fromCodePoint(0x1f600)],
  ["combining mark", `e${String.fromCodePoint(0x0301)}`],
  [
    "ZWJ sequence",
    `${String.fromCodePoint(0x1f469)}${String.fromCodePoint(0x200d)}` +
      String.fromCodePoint(0x1f4bb),
  ],
  ["flag", `${String.fromCodePoint(0x1f1ee)}${String.fromCodePoint(0x1f1f9)}`],
] as const;

test("bounded text never returns a partial grapheme at either edge", () => {
  for (const [name, cluster] of clusters) {
    const text = `ab${cluster}cd`;
    const budget = cluster.length + 1;
    const leading = leadingText(text, budget);
    const trailing = trailingText(text, budget);

    assert.equal(leading, "ab", `${name} was split at the leading boundary`);
    assert.equal(trailing, "cd", `${name} was split at the trailing boundary`);
    assert.ok(leading.length <= budget);
    assert.ok(trailing.length <= budget);
    assert.equal(leading.isWellFormed(), true);
    assert.equal(trailing.isWellFormed(), true);
    assert.equal(leadingText(text, cluster.length + 2), `ab${cluster}`);
    assert.equal(trailingText(text, cluster.length + 2), `${cluster}cd`);
  }
});

test("bounded text keeps complete input and honors an empty budget", () => {
  assert.equal(leadingText("complete", 20), "complete");
  assert.equal(trailingText("complete", 20), "complete");
  assert.equal(leadingText("complete", 0), "");
  assert.equal(trailingText("complete", 0), "");
});
