import { test } from "node:test";
import assert from "node:assert/strict";
import { defaultTranscriptName, transcriptMarkdown } from "../src/transcript.ts";
import type { Block } from "../src/tui/blocks.ts";

test("the default transcript name is portable and sortable", () => {
  assert.equal(
    defaultTranscriptName(new Date("2026-08-29T12:34:56.789Z")),
    "jecode-transcript-20260829T123456789Z.md",
  );
});

test("exports created in the same second receive distinct names", () => {
  assert.notEqual(
    defaultTranscriptName(new Date("2026-08-29T12:34:56.001Z")),
    defaultTranscriptName(new Date("2026-08-29T12:34:56.999Z")),
  );
});

test("a transcript preserves conversation, reasoning, and full tool details", () => {
  const blocks: Block[] = [
    { kind: "user", text: "Fix it" },
    { kind: "reasoning", text: "Inspect first", expanded: false },
    {
      kind: "tool",
      name: "edit_file",
      target: "src/a.ts",
      right: "1 replacement",
      tone: "ok",
      body: [
        { kind: "del", text: "old" },
        { kind: "add", text: "new" },
      ],
    },
    { kind: "answer", text: "Done." },
  ];

  const markdown = transcriptMarkdown(blocks);
  assert.match(markdown, /## You\n\nFix it/);
  assert.match(markdown, /<summary>Reasoning<\/summary>/);
  assert.match(markdown, /\*\*edit_file\*\* `src\/a\.ts`/);
  assert.match(markdown, /- old\n\+ new/);
  assert.match(markdown, /## Assistant\n\nDone\./);
});

test("an exported transcript keeps terminal controls inert", () => {
  const escape = String.fromCharCode(27);
  const bell = String.fromCharCode(7);
  const blocks: Block[] = [
    { kind: "answer", text: `before${escape}]52;c;payload${bell}after\nsecond line` },
    {
      kind: "tool",
      name: `run${escape}_command`,
      target: "fixture",
      right: "exit 0",
      tone: "ok",
      body: [{ kind: "out", text: "reversed\u202efile" }],
    },
  ];

  const markdown = transcriptMarkdown(blocks);
  assert.equal(markdown.includes(escape), false);
  assert.equal(markdown.includes(bell), false);
  assert.match(markdown, /before␛\]52;c;payload␇after\nsecond line/);
  assert.match(markdown, /reversed\\u202efile/);
});
