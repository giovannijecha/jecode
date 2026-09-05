import { test } from "node:test";
import assert from "node:assert/strict";
import { COMMANDS } from "../src/commands.ts";
import type { Key } from "../src/tui/keys.ts";
import type { Lab } from "../dev/tui/controller.ts";
import { createLab } from "../dev/tui/controller.ts";
import { composeLab } from "../dev/tui/view.ts";
import { SCENARIOS } from "../dev/tui/registry.ts";
import { WORKFLOW_MOMENTS, WORKFLOW_TIMES } from "../dev/tui/scenarios/workflow.ts";
import { STEEL } from "../src/ui/theme.ts";

function lab(scene: string): Lab {
  return createLab({ scene, palette: STEEL, expanded: true, selected: 0, tick: 0 });
}

function key(name: string, text = "", ctrl = false): Key { return { name, text, ctrl }; }
const size = { cols: 100, rows: 24 };

function picker(current: Lab) {
  const modal = current.view().modal;
  assert.equal(modal?.kind, "pick");
  if (modal?.kind !== "pick") throw new Error("expected a picker");
  return modal.picker;
}

test("the interactive command menu reaches the complete production catalogue", () => {
  const current = lab("golden");
  const reached = new Set<string>();
  for (let index = 0; index < COMMANDS.length; index++) {
    const view = current.view();
    const command = view.menu?.[view.menuIndex ?? 0];
    assert.ok(command !== undefined);
    reached.add(command.name);
    current.handle(key("down"));
  }
  assert.equal(reached.size, COMMANDS.length);
  assert.equal(current.view().menuIndex, 0);
  current.handle(key("tab"));
  assert.equal(current.view().editor.text, "/help");
  assert.deepEqual(current.view().menu, []);
  current.handle(key("enter"));
  assert.equal(current.state.scene, "help");
  current.handle(key("escape"));
  assert.equal(current.view().modal, undefined);
  current.close();
});

test("settings selection wraps on actual options without a phantom row", () => {
  const current = lab("menu-settings");
  const count = picker(current).options.length;
  assert.equal(count, 6);
  const reached = new Set<number>();
  for (let index = 0; index < count; index++) {
    reached.add(picker(current).index);
    current.handle(key("down"));
  }
  assert.equal(reached.size, count);
  assert.equal(picker(current).index, 0);
  current.close();
});

test("search and provider navigation use production picker input", () => {
  const current = lab("menu-search");
  current.handle(key("char", "ude-opus"));
  assert.match(picker(current).options[picker(current).index]?.label ?? "", /opus/);
  current.handle(key("char", "no-match"));
  current.handle(key("enter"));
  assert.ok(current.view().modal !== undefined, "an empty search must not select a row");
  current.select("menu-providers");
  current.handle(key("enter"));
  assert.equal(current.state.scene, "providers-account");
  current.select("menu-providers");
  current.handle(key("down"));
  current.handle(key("enter"));
  assert.equal(current.state.scene, "providers-api");
  assert.equal(picker(current).options.length, 3);
  current.close();
});

test("preview fields preserve grapheme editing and never expose the synthetic secret", () => {
  const current = lab("field");
  current.handle(key("u", "", true));
  current.handle(key("paste", "a👩‍💻é"));
  current.handle(key("backspace"));
  const modal = current.view().modal;
  assert.equal(modal?.kind, "type");
  if (modal?.kind !== "type") throw new Error("expected a field");
  assert.equal(modal.field.editor.text, "a👩‍💻");
  assert.doesNotMatch(current.render(size).rows.join("\n"), /👩|masked-demo-value/);
  current.handle(key("enter"));
  assert.equal(current.view().modal, undefined);
  assert.doesNotMatch(JSON.stringify(current.view().blocks), /👩|masked-demo-value/);
  current.close();
});

test("readiness failure retains the draft through the real submit path", () => {
  const current = lab("feedback");
  const draft = current.view().editor.text;
  current.handle(key("enter"));
  assert.equal(current.view().editor.text, draft);
  assert.equal(current.view().blocks.length, 0);
  assert.match(current.render(size).rows.at(-1) ?? "", /needs an API key/);
  current.close();
});

test("fixture time resets on navigation and restart, while detail expansion survives updates", () => {
  const current = lab("tools-stream");
  const initial = structuredClone(current.view().blocks);
  current.advance(2_400);
  assert.match(current.view().status ?? "", /2s/);
  assert.notDeepEqual(current.view().blocks, initial);
  const tool = current.view().blocks[1];
  current.handle(key("o", "", true));
  current.advance();
  assert.equal(current.view().blocks[1], tool, "live blocks retain their identity");
  assert.equal(tool?.kind === "tool" && tool.expanded, false);
  current.setPlaying(false);
  current.advance();
  assert.equal(current.state.tick * 80, 2_560, "manual stepping works while paused");
  current.navigate(1);
  current.select("tools-stream");
  assert.equal(current.state.tick, 0);
  assert.deepEqual(current.view().blocks, initial);
  current.advance(800);
  current.restart();
  assert.equal(current.state.tick, 0);
  assert.throws(() => current.advance(-1), /invalid lab time/);
  current.close();
});

test("waiting, running and settled tool states have an explicit fixture timeline", () => {
  const current = lab("tools-lifecycle");
  const tool = () => {
    const block = current.view().blocks[1];
    assert.equal(block?.kind, "tool");
    if (block?.kind !== "tool") throw new Error("expected tool evidence");
    return block;
  };
  assert.equal(tool().right, "waiting");
  assert.equal(tool().startedAt, undefined);
  current.advance(800);
  assert.equal(tool().right, "running");
  assert.equal(tool().startedAt, 800);
  current.advance(2_400);
  assert.equal(tool().tone, "ok");
  assert.equal(tool().durationMs, 2_400);
  assert.equal(tool().startedAt, undefined);
  assert.equal(current.view().status, undefined);
  current.close();
});

test("queued guidance returns to the composer on interruption and fixture completion", () => {
  for (const scene of ["steering", "tools-lifecycle"]) {
    const current = lab(scene);
    if (scene === "tools-lifecycle") current.handle(key("char", "Keep the change focused"));
    const draft = current.view().editor.text;
    const blocks = structuredClone(current.view().blocks);
    current.handle(key("enter"));
    assert.equal(current.view().editor.text, "");
    assert.equal(current.view().steering, 1);
    assert.deepEqual(current.view().blocks, blocks, "guidance does not fabricate a controller turn");
    if (scene === "steering") current.handle(key("escape"));
    else current.advance(3_200);
    assert.equal(current.view().status, undefined);
    assert.equal(current.view().steering, undefined);
    assert.equal(current.view().editor.text, draft);
    if (scene === "steering") {
      const interrupted = structuredClone(current.view().blocks);
      current.advance(800);
      assert.deepEqual(current.view().blocks, interrupted);
    }
    current.close();
  }
});

test("scrolling preserves the visible transcript while new tool output arrives", () => {
  const current = lab("tools-stream");
  const viewport = { cols: 50, rows: 14 };
  current.render(viewport);
  current.handle(key("pageup"));
  current.render(viewport);
  const before = current.view().scroll;
  assert.ok(before > 0);
  current.advance(2_400);
  current.render(viewport);
  assert.ok(current.view().scroll > before, "growing output preserves the distance from the beginning");
  current.handle(key("pagedown"));
  assert.ok(current.view().scroll >= 0);
  current.select("scroll");
  const tail = current.render(size).rows.join("\n");
  current.handle(key("pageup"));
  assert.notEqual(current.render(size).rows.join("\n"), tail);
  current.close();
});

test("headless and interactive previews share rendering, including reduced motion", () => {
  const initial = { scene: "tools-stream", palette: STEEL, selected: 0, tick: 30, expanded: true };
  const current = createLab(initial);
  const frame = current.render(size);
  assert.deepEqual(composeLab(initial, size), { rows: frame.rows, cursor: frame.cursor });
  current.setReducedMotion(true);
  assert.equal(current.view().reducedMotion, true);
  assert.equal(current.render(size).transcriptAnimating, false);
  current.close();
});

test("workflow evidence expands through production input and retains context while advancing", () => {
  const current = lab("tools-workflow");
  try {
    current.advance(WORKFLOW_TIMES.editStartedMs + 800);
    const tool = current.view().blocks.find((block) => block.kind === "tool" && block.name === "edit_file");
    assert.ok(tool?.kind === "tool");
    assert.equal(tool.expanded, false);
    assert.match(current.render(size).rows.join("\n"), /more changed lines/);
    current.handle(key("o", "", true));
    assert.equal(tool.expanded, true);
    current.advance();
    assert.ok(current.view().blocks.includes(tool), "expansion and clock updates keep the same block");
    assert.equal(tool.expanded, true);
    let foundContext = false;
    for (let page = 0; page < 30; page++) {
      if (current.render(size).rows.some((line) => line.includes("type RetryContext"))) {
        foundContext = true;
        break;
      }
      current.handle(key("pageup"));
    }
    assert.ok(foundContext, "Ctrl+O and scrolling expose unchanged source context");
  } finally {
    current.close();
  }
});

test("seeking and playback produce the same frames through tool arrival and settlement", () => {
  for (const scene of SCENARIOS.filter((item) => "animated" in item)) {
    const current = lab(scene.id);
    let previous = 0;
    const times = [0, 123, 400, 800, 1_090, 1_170, 1_200, 2_400, 3_200, 3_201, 4_800, 8_000,
      ...(scene.id === "tools-workflow" ? WORKFLOW_MOMENTS.map(({ time }) => time) : [])];
    for (const time of times) {
      for (let at = previous; at < time; at += 80) {
        current.advance(Math.min(80, time - at));
        current.render(size);
      }
      const frame = current.render(size);
      const captured = composeLab({
        scene: scene.id, palette: STEEL, tick: time / 80, selected: 0, expanded: true,
      }, size);
      assert.deepEqual(captured.rows, frame.rows, `${scene.id} at ${time}ms`);
      previous = time;
    }
    current.close();
  }
});
