import { test } from "node:test";
import assert from "node:assert/strict";
import { textWidth } from "../src/ui/width.ts";
import { composeLab, SCENES } from "../dev/tui-lab/view.ts";
import type { LabState, Scene } from "../dev/tui-lab/view.ts";
import { STEEL } from "../src/ui/theme.ts";

const ESC = String.fromCharCode(27);
const ANSI = new RegExp(ESC + "\\[[0-9;]*m", "g");

function state(scene: Scene): LabState {
  return { scene, palette: STEEL, expanded: true, selected: 0, tick: 3 };
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
        assert.ok(textWidth(line) <= size.cols, `${scene} exceeds ${size.cols} cells: ${line}`);
      }
    }
  }
});

test("the golden frame carries the complete production hierarchy", () => {
  const frame = composeLab(state("golden"), { rows: 32, cols: 100 });
  const shown = plain(frame.rows).join("\n");
  assert.match(shown, /Harden the OpenAI retry path/);
  assert.match(shown, /thought/);
  assert.match(shown, /✓ read_file\s+src\/providers\/http\.ts/);
  assert.match(shown, /✓ search_text/);
  assert.match(shown, /The request path is mapped/);
  assert.match(shown, /\/help/);
  assert.match(shown, /claude-sonnet-5 · high · ~\/Codex\/jecode \(main\)/);
  assert.doesNotMatch(shown, /jecode v|Workspace:|execution thread|lab ·/);
});

test("conversation keeps the quiet Jecode identity", () => {
  const shown = plain(composeLab(state("conversation"), { rows: 24, cols: 100 }).rows).join("\n");
  assert.match(shown, /Harden the OpenAI retry path/);
  assert.match(shown, /204 tests complete/);
  assert.match(shown, /~\/Codex\/jecode \(main\)/);
  assert.doesNotMatch(shown, /you ›|jecode ›|[├└]/);
});

test("command output uses a diagnostic tail while expanded output stays complete", () => {
  const expanded = plain(composeLab(state("tools-trace"), { rows: 36, cols: 100 }).rows).join("\n");
  const collapsed = plain(
    composeLab({ ...state("tools-trace"), expanded: false }, { rows: 36, cols: 100 }).rows,
  ).join("\n");

  assert.match(expanded, /run_command\s+node --test test\/http\.test\.ts/);
  assert.match(expanded, /AssertionError/);
  assert.match(expanded, /failed · 612ms/);
  assert.match(collapsed, /diagnostic tail/);
  assert.doesNotMatch(collapsed, /AssertionError/);
  assert.match(collapsed, /earlier lines · ctrl\+o expand/);
  assert.match(expanded, /search_text\s+"signal\.aborted"/);
  assert.match(expanded, /running · 4\.4s/);
});

test("live command output is bounded to the newest rows", () => {
  const shown = plain(
    composeLab({ ...state("tools-stream"), tick: 30, expanded: false }, { rows: 30, cols: 100 }).rows,
  ).join("\n");
  assert.match(shown, /run_command\s+node --test/);
  assert.match(shown, /lines so far/);
  assert.match(shown, /# duration_ms 1398/);
  assert.doesNotMatch(shown, /TAP version 13/);
  assert.match(shown, /esc to interrupt/);
});

test("long diffs keep changed hunks rather than becoming an output tail", () => {
  const shown = plain(
    composeLab({ ...state("tools-diff"), expanded: false }, { rows: 34, cols: 100 }).rows,
  ).join("\n");
  assert.match(shown, /edit_file\s+src\/tools\/shell\.ts/);
  assert.match(shown, /OUTPUT_CAP/);
  assert.match(shown, /return capped/);
  assert.match(shown, /lines hidden · ctrl\+o expand/);
  assert.doesNotMatch(shown, /earlier lines/);
});

test("edit approval and its exact diff remain visible together", () => {
  const shown = plain(composeLab(state("approve-edit"), { rows: 30, cols: 100 }).rows).join("\n");
  assert.match(shown, /◌ edit_file\s+src\/providers\/http\.ts/);
  assert.match(shown, /boundedText/);
  assert.match(shown, /Allow this edit\?/);
  assert.match(shown, /Yes, once/);
  assert.match(shown, /Yes, this file for the session/);
});

test("approval selection keeps an arrow fallback without colour", () => {
  const first = plain(composeLab(state("approve-edit"), { rows: 30, cols: 100 }).rows);
  const second = plain(
    composeLab({ ...state("approve-edit"), selected: 1 }, { rows: 30, cols: 100 }).rows,
  );
  assert.match(first.find((line) => line.includes("Yes, once")) ?? "", /→/);
  assert.doesNotMatch(second.find((line) => line.includes("Yes, once")) ?? "", /→/);
  assert.match(second.find((line) => line.includes("this file")) ?? "", /→/);
});

test("a narrow command approval never exceeds or hides the question", () => {
  for (const cols of [38, 39, 40, 41, 42, 43]) {
    const rows = plain(composeLab(state("approve-command"), { rows: 18, cols }).rows);
    assert.ok(rows.some((line) => line.includes("Run this command?")), `${cols}: question missing`);
    assert.ok(rows.every((line) => textWidth(line) <= cols), `${cols}: row overflow`);
  }
});

test("command autocomplete shares the composer and carries its count", () => {
  const frame = composeLab(state("menu-commands"), { rows: 24, cols: 100 });
  const rows = plain(frame.rows);
  const input = frame.cursor?.row ?? -1;
  assert.match(rows[input] ?? "", /^\/.*1–4 \/ 12$/);
  assert.equal(rows.findIndex((line) => line.includes("/help")), input + 1);
  assert.match(rows[input - 1] ?? "", /^─+$/);
  assert.match(rows[input + 5] ?? "", /^─+$/);
  assert.equal(frame.cursor?.col, 1);
});

test("searchable pickers expose the shared arrow prompt and a real caret", () => {
  const frame = composeLab(state("menu-search"), { rows: 24, cols: 100 });
  const rows = plain(frame.rows);
  const at = frame.cursor;
  assert.ok(at !== undefined);
  assert.match(rows[at.row] ?? "", /^→ cla.*1–3 \/ 3 · 5 total$/);
  assert.equal(at.col, "→ cla".length);
  assert.match(rows[at.row + 1] ?? "", /→ claude-sonnet-5/);
});

test("credential input uses the same writable prompt", () => {
  const frame = composeLab(state("field"), { rows: 24, cols: 100 });
  const rows = plain(frame.rows);
  const title = rows.findIndex((line) => line.includes("paste key"));
  const secret = rows.findIndex((line) => line.includes("●"));
  assert.match(rows[title - 1] ?? "", /^─+$/);
  assert.equal(secret, title + 1);
  assert.match(rows[secret] ?? "", /^→ /);
  assert.equal(frame.cursor?.col, "→ ".length + "masked-demo-value".length);
});

test("settings is a compact selector without key legends", () => {
  const frame = composeLab(state("menu-settings"), { rows: 24, cols: 100 });
  const shown = plain(frame.rows).join("\n");
  assert.match(shown, /settings.*\.jecode\/settings\.json/);
  assert.match(shown, /provider.*ollama/);
  assert.match(shown, /ollama connection.*cloud.*ollama\.com/);
  assert.doesNotMatch(shown, /in use|↑↓ enter|Enter to select/);
  assert.equal(frame.cursor, undefined);
});

test("reasoning stays a bounded unframed live viewport", () => {
  const compact = plain(
    composeLab({ ...state("reasoning"), expanded: false }, { rows: 24, cols: 100 }).rows,
  ).join("\n");
  const expanded = plain(
    composeLab({ ...state("reasoning"), expanded: true }, { rows: 30, cols: 100 }).rows,
  ).join("\n");
  assert.match(compact, /thinking/);
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
