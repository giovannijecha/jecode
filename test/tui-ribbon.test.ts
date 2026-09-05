import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { createLab } from "../dev/tui/controller.ts";
import type { Lab } from "../dev/tui/controller.ts";
import { createPreview } from "../dev/tui/preview.ts";
import { TICK_MS } from "../dev/tui/model.ts";
import { MENU_MOMENTS, menuScene } from "../dev/tui/scenarios/menu-workflow.ts";
import { strip } from "../dev/test-support/tui.ts";
import { COMMANDS } from "../src/commands.ts";
import type { Key } from "../src/tui/keys.ts";
import { compose } from "../src/tui/view.ts";
import { STEEL } from "../src/ui/theme.ts";
import { textWidth } from "../src/ui/width.ts";

const initial = { palette: STEEL, selected: 0, expanded: false, tick: 0, scene: "menu-workflow" };
const sizes = [{ cols: 38, rows: 14 }, { cols: 100, rows: 30 }, { cols: 160, rows: 40 }];
const key = (name: string, text = "", ctrl = false): Key => ({ name, text, ctrl });
const lab = (time = 0): Lab => createLab({ ...initial, tick: time / TICK_MS });

function picker(current: Lab) {
  const modal = current.view().modal;
  assert.ok(modal?.kind === "pick");
  return modal.picker;
}

test("menu navigation preserves the command draft and searchable option identity", (context) => {
  const commands = lab();
  context.after(() => commands.close());
  const draft = commands.view().editor;
  commands.handle(key("down"));
  commands.handle(key("down"));
  assert.equal(commands.view().menu?.length, COMMANDS.length);
  assert.equal(commands.view().editor, draft);
  assert.equal(commands.view().menuIndex, 2);
  const selected = COMMANDS[2]!.name;
  assert.ok(strip(commands.render(sizes[1]!).rows).some((row) => row.startsWith(`● /${selected}`)));
  const models = lab(1_000);
  context.after(() => models.close());
  models.handle(key("down"));
  assert.equal(picker(models).index, 1);
  models.handle(key("paste", "opus"));
  const filtered = picker(models);
  assert.match(filtered.options[filtered.index]!.label, /opus/);
  for (const size of sizes) {
    models.render(size);
    assert.equal(picker(models), filtered);
    assert.equal(picker(models).query, "opus");
  }
  models.handle(key("paste", "-no-such-model"));
  assert.match(strip(models.render(sizes[1]!).rows).join("\n"), /no matches/i);
  models.handle(key("enter"));
  assert.ok(models.view().modal !== undefined, "an empty search cannot settle a hidden choice");
});

test("commands open their production menu workflow sample", (context) => {
  const current = lab();
  context.after(() => current.close());
  for (const [command, time, kind] of [["models", 1_000, "pick"], ["settings", 2_000, "pick"],
    ["permissions", 3_000, "pick"], ["help", 7_000, "help"]] as const) {
    current.handle(key("escape"));
    current.handle(key("u", "", true));
    current.handle(key("paste", `/${command}`));
    current.handle(key("enter"));
    assert.equal(current.state.scene, "menu-workflow");
    assert.equal(current.state.tick * TICK_MS, time, command);
    assert.equal(current.view().modal?.kind, kind, command);
  }
});

test("approval rows preserve once, session and deny indices without executing the fixture", (context) => {
  for (const time of [4_000, 5_000]) for (const index of [0, 1, 2, undefined]) {
    const selected: number[] = [];
    const preview = createPreview(menuScene({ ...initial, tick: time / TICK_MS }), {
      command() { assert.fail("approval must not dispatch a command"); },
      pick(value) { selected.push(value); },
    });
    context.after(() => preview.close());
    const modal = preview.view().modal;
    assert.ok(modal?.kind === "pick");
    assert.deepEqual(modal.picker.options.map((option) => option.key), ["y", "a", "n"]);
    const evidence = structuredClone(preview.view().blocks);
    if (index === undefined) preview.handle(key("escape"));
    else {
      for (let step = 0; step < index; step++) preview.handle(key("down"));
      const label = modal.picker.options[index]!.label;
      assert.ok(strip(preview.render(sizes[1]!).rows).some((row) => row.startsWith(`● ${label}`)));
      preview.handle(key("enter"));
    }
    assert.deepEqual(selected, index === undefined ? [] : [index]);
    assert.equal(preview.view().modal, undefined);
    assert.deepEqual(preview.view().blocks, evidence);
    assert.equal(preview.exited, false);
  }
});

test("permission adjustments retain their selected row and changed policy across redraws", (context) => {
  const current = lab(3_000);
  context.after(() => current.close());
  const index = picker(current).options.findIndex((option) => option.label === "edit_file");
  assert.ok(index >= 0);
  current.handle(key("home"));
  for (let step = 0; step < index; step++) current.handle(key("down"));
  current.handle(key("right"));
  const changed = picker(current);
  assert.equal(changed.options[index]?.value, "allow");
  assert.equal(changed.options[index]?.hint, undefined);
  for (const size of sizes) {
    current.render(size);
    assert.equal(picker(current), changed);
    assert.equal(picker(current).index, index);
  }
  current.handle(key("right"));
  assert.equal(picker(current).options[index]?.value, "deny");
});

test("closing commands returns the full-width composer and grapheme caret", (context) => {
  const current = lab();
  context.after(() => current.close());
  current.handle(key("escape"));
  assert.deepEqual(current.view().menu, []);
  assert.equal(current.view().editor.text, "/");
  current.handle(key("u", "", true));
  const draft = "保持 👩‍💻 é ".repeat(20);
  current.handle(key("paste", draft));
  current.handle(key("left"));
  for (const size of sizes) {
    const actual = current.render(size);
    const expected = compose(current.view(), size);
    assert.deepEqual(actual.rows, expected.rows);
    assert.deepEqual(actual.cursor, expected.cursor);
    assert.equal(current.view().editor.text, draft);
  }
});

test("masked fields retain their editor without exposing its contents", (context) => {
  const current = lab(6_000);
  context.after(() => current.close());
  current.handle(key("u", "", true));
  const secret = "synthetic-value-👩‍💻-é";
  current.handle(key("paste", secret));
  const modal = current.view().modal;
  assert.ok(modal?.kind === "type");
  const field = modal.field;
  for (const size of sizes) {
    const frame = current.render(size);
    const retained = current.view().modal;
    assert.ok(retained?.kind === "type");
    assert.equal(retained.field, field);
    assert.equal(retained.field.editor.text, secret);
    assert.doesNotMatch(frame.rows.join("\n"), /synthetic-value|masked-demo-value|👩/);
    assert.deepEqual(frame, compose(current.view(), size));
  }
  current.handle(key("escape"));
  assert.equal(current.view().modal, undefined);
  assert.doesNotMatch(JSON.stringify(current.view().blocks), /synthetic-value|masked-demo-value|👩/);
});

test("navigating menus keeps dock height and selected labels and values visible at every size", (context) => {
  const current = lab();
  context.after(() => current.close());
  for (const moment of MENU_MOMENTS) {
    assert.equal(current.state.tick * TICK_MS, moment.time);
    const modal = current.view().modal;
    const options = modal?.kind === "pick" ? modal.picker.options : undefined;
    const count = options?.length ?? current.view().menu?.length ?? 0;
    const tops = new Map<number, number>();
    for (let selection = 0; selection < Math.max(1, count); selection++) {
      for (const size of sizes) {
        const frame = current.render(size);
        const rows = strip(frame.rows);
        const text = rows.join(" ").replace(/\s+/gu, " ");
        assert.equal(frame.rows.length, size.rows, moment.title);
        assert.ok(rows.every((row) => textWidth(row) <= size.cols), moment.title);
        const top = rows.indexOf("─".repeat(size.cols));
        if (selection === 0) tops.set(size.cols, top);
        else assert.equal(top, tops.get(size.cols), `${moment.title}: selection must not move the dock`);
        if (count > 0) {
          const label = options?.[selection]?.label ?? `/${current.view().menu![selection]!.name}`;
          assert.ok(rows.some((row) => row.startsWith(`● ${label}`)), `${moment.title}: ${label} at ${size.cols}`);
          const value = options?.[selection]?.value;
          if (value !== undefined) assert.ok(text.includes(value), `${moment.title}: ${value} at ${size.cols}`);
        }
        if (frame.cursor !== undefined) {
          assert.ok(frame.cursor.row >= 0 && frame.cursor.row < size.rows);
          assert.ok(frame.cursor.col >= 0 && frame.cursor.col < size.cols);
        }
      }
      if (count > 0) current.handle(key("down"));
    }
    current.nextMoment();
  }
  assert.equal(current.state.tick, 0);
  assert.equal(current.playing, false);
  assert.equal(current.view().editor.text, "/");
  assert.equal(current.view().menuIndex, 0);
});

test("fresh colour and NO_COLOR keep the same selected dot band and writable query arrow", () => {
  const script = `
    import assert from "node:assert/strict";
    Object.defineProperty(process.stdout, "isTTY", { value: true });
    const { createLab } = await import("./dev/tui/controller.ts");
    const { TICK_MS } = await import("./dev/tui/model.ts");
    const { STEEL } = await import("./src/ui/theme.ts");
    const { hasColor, configureColor } = await import("./src/ui/render.ts");
    const { textWidth } = await import("./src/ui/width.ts");
    const plain = rows => rows.map(row => row.replace(/\\x1b\\[[0-9;]*m/g, ""));
    assert.equal(hasColor(), process.env.NO_COLOR === undefined);
    for (const [scene, time, steps] of [["menu-workflow", 0, 1], ["menu-workflow", 1_000, 1],
      ["menu-workflow", 3_000, 1], ...[1, 2, 3, 4].map(steps => ["menu-timeline", 0, steps])]) {
      const current = createLab({ scene, palette: STEEL, selected: 0, expanded: false, tick: time / TICK_MS });
      try {
        for (let step = 0; step < steps; step++) current.handle({ name: "down", text: "", ctrl: false });
        for (const size of [{ cols: 38, rows: 14 }, { cols: 100, rows: 30 }, { cols: 160, rows: 40 }]) {
          configureColor(true);
          current.invalidate();
          const coloured = current.render(size);
          const rows = plain(coloured.rows);
          const selected = rows.findIndex(row => row.startsWith("● "));
          assert.ok(selected >= 0);
          assert.equal(rows.filter(row => row.startsWith("● ")).length, 1);
          assert.equal(textWidth(rows[selected]), size.cols, "the band fills the available width");
          if (hasColor()) assert.ok(coloured.rows[selected].includes("\\x1b[48;2;" + STEEL.surface.subtle.join(";") + "m"));
          if (time === 1_000) assert.ok(rows.some(row => row.startsWith("→ ")), "the search prompt keeps its arrow");
          assert.ok(rows.every(row => textWidth(row) <= size.cols));
          assert.equal(coloured.rows.some(row => row.includes("\\x1b[")), hasColor());
          if (scene === "menu-timeline") {
            const title = rows.findIndex(row => row.startsWith("timeline"));
            const choices = rows.slice(title + 2, -3);
            assert.equal(choices.length, 4, "one row per node, including long selected previews");
            assert.ok(choices.every(row => row.trim() !== ""));
            assert.equal(current.view().modal.picker.index, steps % 4);
            assert.deepEqual(coloured.cursor, { row: title + 1, col: 2 });
          }
          configureColor(false);
          current.invalidate();
          const monochrome = current.render(size);
          // User surfaces use half-cell colour edges and blank NO_COLOR padding.
          assert.deepEqual(monochrome.rows, rows.map(row => /^[▄▀]+$/u.test(row) ? " ".repeat(textWidth(row)) : row));
          assert.deepEqual(monochrome.cursor, coloured.cursor);
        }
        if (scene === "menu-timeline") {
          current.handle({ name: "escape", text: "", ctrl: false });
          assert.equal(current.view().modal, undefined);
        }
      } finally { current.close(); }
    }
  `;
  for (const noColor of [false, true]) {
    const env: NodeJS.ProcessEnv = { ...process.env, TERM: "xterm-256color" };
    if (noColor) env["NO_COLOR"] = "1";
    else delete env["NO_COLOR"];
    delete env["FORCE_COLOR"];
    const result = spawnSync(process.execPath, ["--input-type=module", "--eval", script], {
      cwd: fileURLToPath(new URL("..", import.meta.url)), env, encoding: "utf8", timeout: 15_000,
    });
    assert.equal(result.status, 0, result.stderr);
  }
});
