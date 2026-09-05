import { test } from "node:test";
import assert from "node:assert/strict";
import type { Block } from "../src/tui/blocks.ts";
import { renderAll } from "../src/tui/blocks.ts";
import { reasoningPreviewSource } from "../src/tui/components/messages.ts";
import { STEEL } from "../src/ui/theme.ts";
import { strip } from "../dev/test-support/tui.ts";

test("reasoning keeps an unframed three-row tail and expands without losing text", () => {
  const block: Block = {
    kind: "reasoning",
    text: "- line one\n- line two\n- line three\n- line four",
    live: true,
    expanded: false,
  };
  const preview = strip(renderAll([block], 50, STEEL));
  const shown = preview.join("\n");
  assert.doesNotMatch(shown, /thinking|thought|· live/);
  assert.doesNotMatch(shown, /line one/);
  assert.match(shown, /line two/);
  assert.match(shown, /line three/);
  assert.match(shown, /line four/);
  assert.equal(preview.length, 4, "outer gap and three content rows");
  assert.equal(preview[0], "");
  assert.ok(preview.slice(1).every((line) => line !== " ".repeat(50)));
  assert.doesNotMatch(shown, /[─│]/);

  block.live = false;
  block.expanded = true;
  const full = strip(renderAll([block], 50, STEEL)).join("\n");
  assert.doesNotMatch(full, /thinking|thought|· live/);
  assert.match(full, /line one/);
});

test("compact reasoning bounds the markdown source without discarding the full block", () => {
  const text = `old context ${"analysis ".repeat(20_000)}visible tail`;
  const source = reasoningPreviewSource(text, 80);

  assert.equal(source.truncated, true);
  assert.ok(source.text.length <= 4_096);
  assert.doesNotMatch(source.text, /old context/);
  assert.match(source.text, /visible tail$/);

  const block: Block = { kind: "reasoning", text, live: true };
  const shown = strip(renderAll([block], 80, STEEL)).join("\n");
  assert.match(shown, /visible tail/);
  assert.equal(block.text, text);

  block.live = false;
  const settled = strip(renderAll([block], 80, STEEL)).join("\n");
  assert.doesNotMatch(settled, /old context/);
  assert.match(settled, /visible tail/);
});

test("compact reasoning starts on a complete grapheme boundary", () => {
  const limit = 4_096;
  const clusters = [
    String.fromCodePoint(0x1f600),
    `e${String.fromCodePoint(0x0301)}`,
    `${String.fromCodePoint(0x1f469)}${String.fromCodePoint(0x200d)}` +
      String.fromCodePoint(0x1f4bb),
    `${String.fromCodePoint(0x1f1ee)}${String.fromCodePoint(0x1f1f9)}`,
  ];

  for (const cluster of clusters) {
    const suffix = "a".repeat(limit + 1 - cluster.length);
    const source = reasoningPreviewSource(`x${cluster}${suffix}`, 80);
    assert.equal(source.text, suffix);
    assert.equal(source.text.isWellFormed(), true);
  }
});

test("a live expansion waits for the complete thought before rendering the full source", () => {
  const text = `old context ${"analysis ".repeat(20_000)}visible tail`;
  const block: Block = { kind: "reasoning", text, live: true, expanded: true };

  const live = strip(renderAll([block], 80, STEEL)).join("\n");
  assert.doesNotMatch(live, /old context/);
  assert.match(live, /visible tail/);

  block.live = false;
  const sealed = strip(renderAll([block], 80, STEEL)).join("\n");
  assert.match(sealed, /old context/);
});
