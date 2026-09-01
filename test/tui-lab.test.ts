import { test } from "node:test";
import assert from "node:assert/strict";
import { COMMANDS } from "../src/commands.ts";
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
  assert.match(shown, /inspect the HTTP boundary first/);
  assert.doesNotMatch(shown, /thinking|thought/);
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
  assert.match(shown, /Running run_command · 4s/);
  assert.match(shown, /esc to interrupt/);
});

test("compact diffs show every changed line without context noise", () => {
  const shown = plain(
    composeLab({ ...state("tools-diff"), expanded: false }, { rows: 34, cols: 100 }).rows,
  ).join("\n");
  assert.match(shown, /edit_file\s+src\/tools\/shell\.ts/);
  assert.match(shown, /OUTPUT_CAP/);
  assert.match(shown, /return capped/);
  assert.doesNotMatch(shown, /unchanged|lines hidden|earlier lines/);
  assert.doesNotMatch(shown, /function collect|const text = chunks\.join/);
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
  assert.match(rows[input] ?? "", new RegExp(`^/.*1–4 / ${COMMANDS.length}$`));
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

test("resume is represented by its real searchable session picker", () => {
  const shown = plain(composeLab(state("menu-resume"), { rows: 24, cols: 100 }).rows).join("\n");
  assert.match(shown, /resume\s+saved conversations/);
  assert.match(shown, /Harden durable sessions/);
  assert.match(shown, /12 turns/);
  assert.doesNotMatch(shown, /session-durable|session-footer|session-providers/);
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
  assert.match(shown, /model.*Ollama/);
  assert.match(shown, /providers.*manage.*connections/);
  assert.doesNotMatch(shown, /settings.*\.jecode|Changes apply|ollama connection|authentication/);
  assert.doesNotMatch(shown, /in use|↑↓ enter|Enter to select/);
  assert.equal(frame.cursor, undefined);
});

test("essential control-plane values remain visible at the minimum width", () => {
  const settings = plain(
    composeLab(state("menu-settings"), { rows: 14, cols: 38 }).rows,
  ).join("\n");
  const permissions = plain(
    composeLab(state("menu-permissions"), { rows: 14, cols: 38 }).rows,
  ).join("\n");

  assert.match(settings, /model.*Ollama/);
  assert.match(settings, /effort.*high/);
  assert.match(settings, /max output.*64000/);
  assert.match(permissions, /read_file.*allow/);
});

test("permissions exposes every tool and its session policy", () => {
  const frame = composeLab(state("menu-permissions"), { rows: 24, cols: 100 });
  const shown = plain(frame.rows).join("\n");
  const tail = plain(
    composeLab({ ...state("menu-permissions"), selected: 6 }, { rows: 24, cols: 100 }).rows,
  ).join("\n");
  assert.match(shown, /read_file.*read only.*‹ allow ›/);
  assert.match(shown, /search_text.*read only.*deny/);
  assert.match(shown, /edit_file.*2 remembered.*ask/);
  assert.match(tail, /run_command.*1 remembered.*‹ ask ›/);
  assert.doesNotMatch(shown, /permissions|session only|Changes apply|close|in use|Enter to select/);
  assert.equal(frame.cursor, undefined);
});

test("help is a temporary keyboard reference inside the shared dock", () => {
  const frame = composeLab(state("help"), { rows: 24, cols: 100 });
  const shown = plain(frame.rows).join("\n");
  assert.match(shown, /help\s+keyboard controls.*esc close/);
  assert.match(shown, /ctrl\+left \/ right.*move cursor by word/);
  assert.match(shown, /ctrl\+backspace\/del.*delete a word/);
  assert.match(shown, /alt\+enter\s+insert a new line/);
  assert.match(shown, /ctrl\+l\s+redraw the screen/);
  assert.equal(frame.cursor, undefined);
});

test("reasoning stays bounded and defers full expansion while live", () => {
  const compact = plain(
    composeLab({ ...state("reasoning"), expanded: false }, { rows: 24, cols: 100 }).rows,
  ).join("\n");
  const expanded = plain(
    composeLab({ ...state("reasoning"), expanded: true }, { rows: 30, cols: 100 }).rows,
  ).join("\n");
  assert.doesNotMatch(compact, /thinking|thought|ctrl\+o/);
  assert.doesNotMatch(expanded, /thinking|thought|ctrl\+o/);
  assert.match(compact, /Reasoning can keep its complete source text/);
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
  assert.deepEqual(STEEL.surface.subtle, [31, 38, 47]);
  assert.equal("reasoning" in STEEL.surface, false);
  assert.deepEqual(STEEL.surface.added, [22, 55, 34]);
  assert.deepEqual(STEEL.surface.removed, [62, 24, 27]);
});

test("the lab has an exact-height recovery frame for a tiny terminal", () => {
  const frame = composeLab(state("conversation"), { rows: 6, cols: 24 });
  assert.equal(frame.rows.length, 6);
  assert.match(plain(frame.rows).join("\n"), /too small/);
  assert.equal(frame.cursor, undefined);
});
