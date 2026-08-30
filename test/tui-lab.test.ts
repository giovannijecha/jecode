import { test } from "node:test";
import assert from "node:assert/strict";
import { textWidth } from "../src/ui/width.ts";
import {
  composeLab,
  SCENES,
} from "../dev/tui-lab/view.ts";
import type { LabState, Scene } from "../dev/tui-lab/view.ts";
import { STEEL } from "../src/ui/theme.ts";

const ESC = String.fromCharCode(27);
const ANSI = new RegExp(ESC + "\\[[0-9;]*m", "g");

function state(scene: Scene): LabState {
  return {
    scene,
    palette: STEEL,
    expanded: true,
    selected: 0,
    tick: 3,
  };
}

function plain(rows: readonly string[]): string[] {
  return rows.map((line) => line.replace(ANSI, ""));
}

test("every production-catalogue scene produces a bounded real terminal frame", () => {
  for (const scene of SCENES) {
    for (const size of [
      { rows: 14, cols: 38 },
      { rows: 16, cols: 40 },
      { rows: 24, cols: 80 },
      { rows: 40, cols: 120 },
    ]) {
      const frame = composeLab(state(scene), size);
      assert.equal(frame.rows.length, size.rows, scene);
      for (const line of plain(frame.rows)) {
        assert.ok(
          textWidth(line) <= size.cols,
          scene + " exceeds " + size.cols + " cells: " + line,
        );
      }
    }
  }
});

test("conversation uses the approved hierarchy with Jecode identity", () => {
  const shown = plain(composeLab(state("conversation"), { rows: 24, cols: 100 }).rows).join("\n");
  assert.doesNotMatch(shown, /jecode v|Workspace:|ctrl\+c interrupt/);
  assert.match(shown, /Harden the OpenAI retry path/);
  assert.match(shown, /204 tests complete/);
  assert.match(shown, /~\/Codex\/jecode \(main\)/);
  assert.doesNotMatch(shown, /you ›|jecode ›|execution thread|focus mode|bare transcript|[│├└]/);
  assert.doesNotMatch(shown, /lab ·/);
});

test("tool output expands inside one state-colored tool card", () => {
  const expanded = plain(composeLab(state("tools"), { rows: 32, cols: 100 }).rows).join("\n");
  const collapsed = plain(
    composeLab({ ...state("tools"), expanded: false }, { rows: 32, cols: 100 }).rows,
  ).join("\n");

  assert.match(expanded, /run_command node --test test\/http\.test\.ts/);
  assert.match(expanded, /AssertionError/);
  assert.match(expanded, /Took 612ms/);
  assert.match(collapsed, /AssertionError/);
  assert.match(expanded, /diagnostic tail/);
  assert.doesNotMatch(collapsed, /diagnostic tail/);
  assert.match(expanded, /search_text "signal\.aborted"/);
  assert.match(expanded, /Tracing the failed assertion/);
});

test("diff and production permission selector remain visible together", () => {
  const shown = plain(composeLab(state("diff"), { rows: 30, cols: 100 }).rows).join("\n");
  assert.match(shown, /edit_file src\/providers\/http\.ts/);
  assert.match(shown, /boundedText/);
  assert.match(shown, /Permission required/);
  assert.match(shown, /Run this edit once/);
});

test("permission selection moves the arrow without adding a selected panel", () => {
  const first = plain(composeLab(state("diff"), { rows: 30, cols: 100 }).rows);
  const second = plain(
    composeLab({ ...state("diff"), selected: 1 }, { rows: 30, cols: 100 }).rows,
  );

  assert.match(first.find((line) => line.includes("Run this edit once")) ?? "", /→/);
  assert.doesNotMatch(second.find((line) => line.includes("Run this edit once")) ?? "", /→/);
  assert.match(second.find((line) => line.includes("Allow changes")) ?? "", /→/);
});

test("command autocomplete lives inside the composer rails", () => {
  const frame = composeLab(state("commands"), { rows: 24, cols: 100 });
  const rows = plain(frame.rows);
  const shown = rows.join("\n");
  assert.match(shown, /\/help/);
  assert.match(shown, /\/exit/);
  const input = rows.findIndex((line) => line === "/");
  const firstMatch = rows.findIndex((line) => line.includes("/help"));
  assert.ok(input >= 0);
  assert.equal(firstMatch, input + 1);
  const lastMatch = rows.findIndex((line) => line.includes("/usage"));
  assert.match(rows[input - 1] ?? "", /^─+$/);
  assert.match(rows[lastMatch + 1] ?? "", /^─+$/);
  assert.equal(frame.cursor?.col, 1);
});

test("markdown uses muted fences without the legacy code rail", () => {
  const shown = plain(composeLab(state("markdown"), { rows: 30, cols: 100 }).rows).join("\n");
  assert.match(shown, /```md/);
  assert.match(shown, /```js/);
  assert.match(shown, /  export function total/);
  assert.doesNotMatch(shown, /▌/);
});

test("credential input uses the same compact composer shell", () => {
  const frame = composeLab(state("field"), { rows: 24, cols: 100 });
  const rows = plain(frame.rows);
  const title = rows.findIndex((line) => line.includes("paste key"));
  const secret = rows.findIndex((line) => line.includes("●"));
  assert.match(rows[title - 1] ?? "", /^─+$/);
  assert.equal(secret, title + 1);
  assert.match(rows[secret + 2] ?? "", /^─+$/);
  assert.equal(frame.cursor?.col, "→ ".length + "masked-demo-value".length);
});

test("settings is a compact selector inside the shared dock", () => {
  const frame = composeLab(state("settings"), { rows: 24, cols: 100 });
  const rows = plain(frame.rows);
  const shown = rows.join("\n");
  assert.match(shown, /settings.*\.jecode\/settings\.json/);
  assert.match(shown, /provider.*ollama/);
  assert.match(shown, /ollama connection.*cloud.*ollama\.com/);
  assert.match(shown, /model.*deepseek-v4-flash:0731/);
  assert.match(rows.find((line) => line.includes("provider")) ?? "", /→/);
  assert.doesNotMatch(shown, /in use/);
  assert.equal(frame.cursor, undefined);
});

test("reasoning has one bounded live viewport with an explicit full view", () => {
  const compact = plain(
    composeLab({ ...state("reasoning"), expanded: false }, { rows: 24, cols: 100 }).rows,
  ).join("\n");
  const expanded = plain(
    composeLab({ ...state("reasoning"), expanded: true }, { rows: 30, cols: 100 }).rows,
  ).join("\n");
  assert.match(compact, /thinking/);
  assert.doesNotMatch(compact, /· live|thought/);
  assert.match(compact, /ctrl\+o full/);
  assert.match(expanded, /ctrl\+o compact/);
  assert.doesNotMatch(compact, /─ thinking|thinking ─/);
});

test("operational feedback occupies the footer beside a retained prompt", () => {
  const rows = plain(composeLab(state("feedback"), { rows: 24, cols: 100 }).rows);
  const feedback = rows.findIndex((line) => line.includes("Anthropic needs an API key"));
  const prompt = rows.findIndex((line) => line.includes("keep this prompt in the composer"));
  assert.equal(feedback, rows.length - 1);
  assert.match(rows[prompt - 1] ?? "", /^─+$/);
  assert.match(rows[prompt + 1] ?? "", /^─+$/);
  assert.equal(feedback, prompt + 2);
});

test("the production identity carries the calibrated dark Steel tokens", () => {
  assert.deepEqual(STEEL.surface.subtle, [52, 53, 65]);
  assert.equal("reasoning" in STEEL.surface, false);
  assert.deepEqual(STEEL.surface.added, [40, 50, 40]);
  assert.deepEqual(STEEL.surface.removed, [60, 40, 40]);
});

test("the lab has an exact-height recovery frame for a tiny terminal", () => {
  const frame = composeLab(state("conversation"), { rows: 6, cols: 24 });
  assert.equal(frame.rows.length, 6);
  assert.match(plain(frame.rows).join("\n"), /too small/);
  assert.equal(frame.cursor, undefined);
});
