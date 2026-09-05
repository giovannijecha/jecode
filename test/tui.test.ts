import { test } from "node:test";
import assert from "node:assert/strict";
import * as edit from "../src/tui/editor.ts";
import { compose } from "../src/tui/view.ts";
import { textWidth } from "../src/ui/width.ts";
import { row } from "../src/ui/render.ts";
import { base, strip } from "../dev/test-support/tui.ts";

test("a frame is exactly as tall as the terminal", () => {
  for (const rows of [10, 24, 60]) {
    const frame = compose(base(), { rows, cols: 80 });
    assert.equal(frame.rows.length, rows);
  }
});

test("a right column that fills the row never adds an extra gutter cell", () => {
  const rendered = row(5, [{ text: "left" }], [{ text: "12345" }]);
  assert.equal(textWidth(rendered), 5);
});

test("the composer cursor starts at the terminal's left edge", () => {
  const frame = compose(base(), { rows: 24, cols: 80 });
  // Nineteen transcript rows, one rhythm row, then the composer border.
  assert.deepEqual(frame.cursor, { row: 21, col: 0 });
});

test("the cursor follows the text as the input wraps", () => {
  const text = "x".repeat(100);
  const frame = compose({ ...base(), editor: edit.of(text) }, { rows: 24, cols: 40 });
  // 100 chars over a 40-wide dock is three rows; the cursor is on the last.
  assert.deepEqual(frame.cursor, { row: 21, col: 20 });
});

test("scroll is bounded by how much there is to scroll", () => {
  const view = {
    ...base(),
    blocks: Array.from({ length: 40 }, (_, i) => ({ kind: "answer" as const, text: `line ${i}` })),
    scroll: 9999,
  };
  const frame = compose(view, { rows: 24, cols: 80 });
  // Each assistant block owns a spacer, against a nineteen-row viewport.
  assert.equal(frame.maxScroll, 40 * 2 - (24 - 5));
  assert.match(frame.rows.slice(0, 2).join("\n"), /line 0/);
});

test("the one-line footer keeps the active provider route visible", () => {
  const frame = compose(base(), { rows: 24, cols: 80 });
  const footer = strip(frame.rows)[23] as string;
  assert.match(footer, /Anthropic API · claude-sonnet-5 · high · ~\/Codex\/jecode/);
  assert.doesNotMatch(footer, /cloud|ready|tokens/);
});

test("the footer exposes unseen output while scroll lock is active", () => {
  const frame = compose({ ...base(), unseen: 3 }, { rows: 24, cols: 80 });
  assert.match(strip(frame.rows).join("\n"), /3 new ↓/);
});

test("activity keeps its state, elapsed time, and interrupt hint on the footer", () => {
  const quiet = compose(base(), { rows: 24, cols: 80 });
  const active = compose(
    { ...base(), status: "Responding · 2s" },
    { rows: 24, cols: 80 },
  );
  const blocked = compose(
    {
      ...base(),
      readiness: { text: "Anthropic needs an API key · /providers", tone: "warn" },
    },
    { rows: 24, cols: 80 },
  );
  const activeRows = strip(active.rows);
  const blockedRows = strip(blocked.rows);

  assert.deepEqual(active.cursor, quiet.cursor);
  assert.equal(activeRows.findIndex((line) => line.includes("esc to interrupt")), 23);
  assert.equal(activeRows.findIndex((line) => line.includes("Responding · 2s")), 23);
  assert.equal(blockedRows.findIndex((line) => line.includes("needs an API key")), 23);
  assert.match(activeRows[23] ?? "", /claude-sonnet-5 · high/);
});

test("queued steering remains visible and bounded on narrow monochrome frames", () => {
  const previous = process.env["NO_COLOR"];
  process.env["NO_COLOR"] = "1";
  try {
    for (const cols of [38, 50, 80]) {
      const frame = compose(
        { ...base(), status: "Running edit_file · 8s", steering: 2 },
        { rows: 14, cols },
      );
      assert.equal(frame.rows.length, 14);
      assert.ok(frame.rows.every((line) => textWidth(line) <= cols));
      assert.match(strip(frame.rows).at(-1) ?? "", /2 queued/);
    }
  } finally {
    if (previous === undefined) delete process.env["NO_COLOR"];
    else process.env["NO_COLOR"] = previous;
  }
});

test("urgent feedback outranks activity while informational feedback waits behind it", () => {
  const urgent = strip(compose({
    ...base(),
    status: "Responding · 2s",
    feedback: { text: "request failed", tone: "error" },
  }, { rows: 24, cols: 80 }).rows)[23] ?? "";
  const informational = strip(compose({
    ...base(),
    status: "Responding · 2s",
    feedback: { text: "settings saved", tone: "info" },
  }, { rows: 24, cols: 80 }).rows)[23] ?? "";

  assert.match(urgent, /× request failed/);
  assert.doesNotMatch(urgent, /Responding/);
  assert.match(informational, /Responding · 2s/);
  assert.match(informational, /esc to interrupt/);
  assert.doesNotMatch(informational, /settings saved/);
});

test("every frame matches the real terminal height, including small modals", () => {
  const prompt = {
    title: [],
    options: Array.from({ length: 20 }, (_, index) => ({ label: `model-${index}` })),
    searchable: true,
    query: "",
    index: 0,
  };
  for (const rows of [8, 10, 12]) {
    const frame = compose({ ...base(), modal: { kind: "pick" as const, picker: prompt } }, { rows, cols: 50 });
    assert.equal(frame.rows.length, rows);
  }
});

test("transient feedback stays visible when a command reopens its parent picker", () => {
  const modal = {
    kind: "pick" as const,
    picker: { title: [], options: [{ label: "credentials" }], index: 0 },
  };
  const frame = compose(
    {
      ...base(),
      status: "Running /settings · 1s",
      feedback: { text: "credential saved · ~/.jecode/credentials.json", tone: "info" as const },
      modal,
    },
    { rows: 24, cols: 80 },
  );
  const rows = strip(frame.rows);
  const feedback = rows.findIndex((line) => line.includes("credential saved"));
  const option = rows.findIndex((line) => line.includes("● credentials"));
  assert.equal(feedback, rows.length - 1);
  assert.ok(option < feedback);
  assert.equal(rows.length, 24);

  const quiet = strip(compose({ ...base(), modal }, { rows: 24, cols: 80 }).rows);
  assert.equal(quiet.findIndex((line) => line.includes("● credentials")), option);
});

test("a genuinely tiny terminal gets an exact-height recovery screen", () => {
  const frame = compose(base(), { rows: 4, cols: 20 });
  assert.equal(frame.rows.length, 4);
  assert.match(strip(frame.rows).join("\n"), /too small/);
  assert.equal(frame.cursor, undefined);
});

test("help is a compact dock surface with no transcript caret", () => {
  const frame = compose(
    {
      ...base(),
      modal: { kind: "help" },
    },
    { rows: 24, cols: 80 },
  );

  const shown = strip(frame.rows).join("\n");
  assert.match(shown, /help\s+keyboard controls.*esc close/);
  assert.match(shown, /ctrl\+o\s+expand\/collapse reasoning or evidence/);
  assert.doesNotMatch(shown, /\/usage/);
  assert.equal(frame.cursor, undefined);
  assert.equal(frame.rows.length, 24);
});
