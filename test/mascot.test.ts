import { test } from "node:test";
import assert from "node:assert/strict";
import { textWidth } from "../src/ui/width.ts";
import {
  MASCOT_COLS,
  MASCOT_ROWS,
  mascotState,
  renderMascot,
  type MascotState,
} from "../src/tui/components/mascot.ts";

const STATES: readonly MascotState[] = [
  "idle",
  "thinking",
  "typing",
  "success",
  "warning",
  "error",
];

test("every Jeco pose fits its fixed terminal footprint", () => {
  const poses = STATES.map((state) => renderMascot(state).join("\n"));

  for (const state of STATES) {
    const rows = renderMascot(state);
    assert.equal(rows.length, MASCOT_ROWS, state);
    assert.ok(rows.some((row) => /[▀▄█]/.test(row)), `${state} is empty`);
    assert.ok(rows.every((row) => textWidth(row) <= MASCOT_COLS), `${state} is too wide`);
  }
  assert.equal(new Set(poses).size, STATES.length);
});

test("live poses animate while reduced motion stays static", () => {
  assert.notDeepEqual(renderMascot("thinking", 0), renderMascot("thinking", 4));
  assert.notDeepEqual(renderMascot("typing", 0), renderMascot("typing", 4));
  assert.deepEqual(renderMascot("thinking", 0, true), renderMascot("thinking", 4, true));
  assert.deepEqual(renderMascot("typing", 0, true), renderMascot("typing", 4, true));
});

test("Jeco reflects real activity and the latest outcome", () => {
  assert.equal(mascotState({ blocks: [] }), "idle");
  assert.equal(mascotState({ blocks: [], activityKind: "turn", status: "Thinking" }), "thinking");
  assert.equal(mascotState({ blocks: [], activityKind: "turn", status: "Writing" }), "typing");
  assert.equal(mascotState({ blocks: [], activityKind: "command", status: "Running /models" }), "typing");
  assert.equal(mascotState({
    blocks: [],
    activityKind: "turn",
    status: "Waiting for you",
  }), "warning");
  assert.equal(mascotState({ blocks: [], feedbackTone: "error" }), "error");
  assert.equal(mascotState({ blocks: [], readinessTone: "warn" }), "warning");
  assert.equal(mascotState({ blocks: [], feedbackTone: "info", readinessTone: "warn" }), "warning");
  assert.equal(mascotState({ blocks: [{ kind: "answer", text: "done" }] }), "success");
  assert.equal(mascotState({
    blocks: [{ kind: "tool", name: "write_file", target: "a.ts", right: "failed", tone: "fail" }],
  }), "error");
});
