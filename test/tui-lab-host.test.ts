import { test } from "node:test";
import assert from "node:assert/strict";
import type { AppScreen } from "../src/tui/app.ts";
import type { Cursor, Painter } from "../src/tui/frame.ts";
import { textWidth } from "../src/ui/width.ts";
import { STEEL } from "../src/ui/theme.ts";
import { runLab } from "../dev/tui/host.ts";
import { parseOptions } from "../dev/tui/options.ts";
import { WORKFLOW_MOMENTS } from "../dev/tui/scenarios/workflow.ts";
import { composeLab } from "../dev/tui/view.ts";

function terminal() {
  let feed = (_chunk: string) => {};
  let resize = () => {};
  const calls: string[] = [];
  const frames: { rows: readonly string[]; cursor?: Cursor }[] = [];
  let size = { cols: 100, rows: 30 };
  const screen: AppScreen = {
    size: () => size,
    enter: () => { calls.push("enter"); },
    leave: () => { calls.push("leave"); },
    setReducedMotion: (value) => { calls.push(`motion:${value}`); },
    onInput: (handler) => { feed = handler; return () => { calls.push("stop-input"); }; },
    onResize: (handler) => { resize = handler; return () => { calls.push("stop-resize"); }; },
  };
  const paint: Painter = {
    paint: (rows, cursor) => { frames.push({ rows: [...rows], cursor }); },
    invalidate: () => {},
  };
  return {
    screen, paint, frames, calls,
    feed: (chunk: string) => feed(chunk),
    resize(cols: number, rows: number) { size = { cols, rows }; resize(); },
    last() { return frames.at(-1)!; },
  };
}

test("catalogue controls do not steal preview text and terminal resources are released", async () => {
  const host = terminal();
  const running = runLab(parseOptions(["--scene", "conversation", "--paused"]), host);
  assert.equal(host.last().cursor, undefined);
  host.feed("\r");
  assert.ok(host.last().cursor !== undefined);
  host.feed("qrmcp[]");
  assert.match(host.last().rows.join("\n"), /qrmcp\[\]/);
  assert.ok(!host.calls.includes("leave"));
  host.feed("\x07");
  host.feed("m");
  assert.ok(host.calls.includes("motion:true"));
  host.resize(38, 14);
  assert.equal(host.last().rows.length, 14);
  assert.ok(host.last().rows.every((line) => textWidth(line) <= 38));
  host.resize(12, 2);
  assert.equal(host.last().rows.length, 2);
  host.feed("q");
  await running;
  assert.deepEqual(host.calls.slice(-3), ["stop-input", "stop-resize", "leave"]);
  const count = host.frames.length;
  host.feed("q");
  host.resize(80, 24);
  assert.equal(host.frames.length, count);
});

test("playback follows the fixture clock despite input, and pause permits exact stepping", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const host = terminal();
  const running = runLab(parseOptions(["--scene", "tools-stream"]), host);
  context.mock.timers.tick(40);
  host.feed("\r");
  context.mock.timers.tick(40);
  assert.match(host.last().rows[1] ?? "", /80ms/);
  host.feed("\x07");
  host.feed(" ");
  context.mock.timers.tick(800);
  assert.match(host.last().rows[1] ?? "", /80ms.*paused/);
  host.feed(".");
  assert.match(host.last().rows[1] ?? "", /160ms.*paused/);
  host.feed("r");
  assert.match(host.last().rows[1] ?? "", /0ms.*paused/);
  host.feed("q");
  await running;
  const count = host.frames.length;
  context.mock.timers.tick(1_000);
  assert.equal(host.frames.length, count);
});

test("capped and automatic previews keep the footer at the bottom and align the caret after resize", async () => {
  for (const capped of [true, false]) {
    const host = terminal();
    host.resize(100, 34);
    const running = runLab(parseOptions([
      "--scene", "conversation", "--paused", "--color", "off", ...(capped ? ["--size", "38x14"] : []),
    ]), host);
    try {
      host.feed("\rDraft");
      for (const [cols, rows] of [[100, 34], [100, 42], [60, 17], [38, 17], [100, 34]] as const) {
        host.resize(cols, rows);
        const viewport = { cols: capped ? Math.min(cols, 38) : cols, rows: capped ? 14 : rows - 3 };
        const expected = composeLab({
          scene: "conversation", palette: STEEL, expanded: true, selected: 0, tick: 0,
        }, viewport);
        const frame = host.last();
        assert.equal(frame.rows.length, rows);
        assert.equal(frame.rows.at(-1), expected.rows.at(-1), "the footer occupies the physical bottom row");
        const top = rows - viewport.rows;
        assert.deepEqual(frame.rows.slice(3, top), Array.from({ length: top - 3 }, () => ""));
        assert.ok(frame.rows.slice(top).every((line) => textWidth(line) <= viewport.cols));
        assert.equal(textWidth(frame.rows.at(-2)!), viewport.cols, "the dock retains the requested width");
        assert.ok(expected.cursor !== undefined && frame.cursor !== undefined);
        assert.equal(frame.cursor.row, expected.cursor.row + top);
        assert.equal(frame.cursor.col, 5);
        assert.equal(frame.rows[frame.cursor.row], "Draft", "the caret follows the actual input row");
      }
    } finally {
      host.feed("\x07q");
      await running;
    }
  }
});

test("a painter failure restores the terminal and preserves the error", async () => {
  const host = terminal();
  const error = new Error("paint failed");
  host.paint.paint = () => { throw error; };
  await assert.rejects(runLab(parseOptions([]), host), (actual) => actual === error);
  assert.deepEqual(host.calls, ["enter", "stop-input", "stop-resize", "leave"]);
});

test("workflow samples pause at evidence and leave preview digits and n available for editing", async () => {
  const host = terminal();
  const running = runLab(parseOptions(["--scene", "tools-workflow", "--time", "11200", "--paused"]), host);
  try {
    host.feed("n");
    assert.match(host.last().rows[0] ?? "", /tools-workflow/);
    assert.match(host.last().rows[1] ?? "", /13600ms.*paused.*Failing command/);
    host.feed("\r12345n");
    assert.match(host.last().rows.join("\n"), /12345n/);
    assert.match(host.last().rows[0] ?? "", /tools-workflow/);
    assert.match(host.last().rows[1] ?? "", /13600ms.*paused.*Failing command/);
    host.feed("\x074");
    assert.match(host.last().rows[0] ?? "", /tools-workflow/);
    assert.match(host.last().rows[1] ?? "", /13600ms.*paused.*Failing command/);
    assert.match(host.last().rows.join("\n"), /12345n/);
    host.feed("n");
    assert.match(host.last().rows[1] ?? "", /16800ms.*paused.*Large edit running/);
    assert.doesNotMatch(host.last().rows.join("\n"), /12345n/);
    host.feed("n".repeat(WORKFLOW_MOMENTS.length));
    assert.match(host.last().rows[1] ?? "", /16800ms.*paused.*Large edit running/);
  } finally {
    if (host.last().cursor !== undefined) host.feed("\x07");
    host.feed("q");
    await running;
  }
});

test("abort releases input, resize and playback resources", async () => {
  const host = terminal();
  const control = new AbortController();
  const running = runLab(parseOptions(["--scene", "tools-stream"]), { ...host, signal: control.signal });
  const error = new Error("stop the lab");
  control.abort(error);
  await assert.rejects(running, (actual) => actual === error);
  assert.deepEqual(host.calls, ["enter", "stop-input", "stop-resize", "leave"]);
});
