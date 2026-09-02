import { test } from "node:test";
import assert from "node:assert/strict";
import { graphemeCeiling, graphemeFloor } from "../src/text-boundary.ts";
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

test("arbitrary offsets snap outward to complete grapheme boundaries", () => {
  for (const [name, cluster] of clusters) {
    const text = `a${cluster}b`;
    const inside = 1 + Math.max(1, cluster.length - 1);
    assert.equal(graphemeFloor(text, inside), 1, `${name} floor was not safe`);
    assert.equal(graphemeCeiling(text, inside), 1 + cluster.length, `${name} ceiling was not safe`);
    assert.equal(graphemeFloor(text, 1 + cluster.length), 1 + cluster.length);
    assert.equal(graphemeCeiling(text, 1), 1);
  }
});

test("a trailing projection does not walk the hidden prefix", () => {
  const prototype = Object.getPrototypeOf(
    new Intl.Segmenter(undefined, { granularity: "grapheme" }).segment(""),
  ) as object;
  const descriptor = Object.getOwnPropertyDescriptor(prototype, Symbol.iterator);
  assert.ok(descriptor?.value !== undefined);

  Object.defineProperty(prototype, Symbol.iterator, {
    ...descriptor,
    value: () => {
      throw new Error("unexpected full grapheme scan");
    },
  });
  try {
    assert.equal(trailingText(`${"hidden ".repeat(100_000)}visible`, 7), "visible");
  } finally {
    Object.defineProperty(prototype, Symbol.iterator, descriptor);
  }
});
