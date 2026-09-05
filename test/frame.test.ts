import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { painter } from "../src/tui/frame.ts";
import { CSI } from "../src/ui/render.ts";

function output() {
  const writes: string[] = [];
  let ready = true;
  return {
    writes,
    block(value: boolean) { ready = !value; },
    sink: { write: (text: string) => { writes.push(text); }, ready: () => ready, onReady: () => () => {} },
  };
}

test("the frame sink receives one differential write and clears removed rows", () => {
  const { sink, writes } = output();
  const paint = painter(sink);
  paint.paint(["first", "second"], { row: 1, col: 3 });
  assert.equal(writes.length, 1);
  assert.ok(writes[0]?.includes(`${CSI}1;1H${CSI}2Kfirst`));
  paint.paint(["first", "changed"]);
  assert.equal(writes.length, 2);
  assert.ok(!writes[1]?.includes("first"));
  assert.ok(writes[1]?.includes("changed"));
  paint.paint(["first"]);
  assert.ok(writes[2]?.includes(`${CSI}2;1H${CSI}2K`));
  assert.ok(!writes[2]?.includes("first"));
  paint.invalidate();
  paint.paint(["first"]);
  assert.ok(writes[3]?.includes("first"));
});

test("thousands of blocked frames produce no queued writes or false diff base", () => {
  const target = output();
  const paint = painter(target.sink);
  paint.paint(["accepted", "remove after resize"]);
  target.block(true);
  for (let i = 0; i < 10_000; i++) paint.paint([`stale ${i}`], { row: 0, col: 1 });
  assert.equal(target.writes.length, 1);
  target.block(false);
  paint.paint(["latest"], { row: 0, col: 4 });
  assert.equal(target.writes.length, 2);
  const actual = target.writes[1] ?? "";
  assert.match(actual, /latest/);
  assert.doesNotMatch(actual, /stale/);
  assert.ok(actual.includes(`${CSI}2;1H${CSI}2K`));
  assert.ok(actual.includes(`${CSI}1;5H`));
});

test("invalidation during blocked output repaints the latest full frame", () => {
  const target = output();
  const paint = painter(target.sink);
  paint.paint(["unchanged"]);
  target.block(true);
  paint.invalidate();
  paint.paint(["discard"]);
  target.block(false);
  paint.paint(["unchanged"]);
  assert.equal(target.writes.length, 2);
  assert.match(target.writes[1] ?? "", /unchanged/);
});

test("the default app painter honors real Writable pressure and removes its drain listener", () => {
  const source = `
    import assert from 'node:assert/strict';
    import { Writable } from 'node:stream';
    const chunks = [];
    const callbacks = [];
    const output = new Writable({ highWaterMark: 1, write(chunk, _encoding, callback) {
      chunks.push(chunk.toString()); callbacks.push(callback);
    } });
    output.isTTY = true;
    const original = Object.getOwnPropertyDescriptor(process, 'stdout');
    Object.defineProperty(process, 'stdout', { value: output, configurable: true });
    const { runApp } = await import(${JSON.stringify(new URL("../src/tui/app.ts", import.meta.url).href)});
    const { session } = await import(${JSON.stringify(new URL("../dev/test-support/app.ts", import.meta.url).href)});
    const { waitFor, delay } = await import(${JSON.stringify(new URL("../dev/test-support/app-harness.ts", import.meta.url).href)});
    let feed;
    const shutdown = new AbortController();
    const running = runApp(session(), process.cwd(), { shutdownSignal: shutdown.signal, screen: {
      size: () => ({cols: 60, rows: 20}), enter() {}, leave() {}, setReducedMotion() {},
      onInput(handler) { feed = handler; return () => {}; }, onResize: () => () => {},
    } });
    try {
      await waitFor(() => feed !== undefined, 'input');
      const queued = output.writableLength;
      assert.ok(queued > 0);
      for (let i = 0; i < 20; i++) feed('\\u0015stale-' + i);
      feed('\\u0015latest-draft');
      await delay(80);
      assert.equal(output.writableLength, queued, 'frames accumulated during pressure');
      callbacks.shift()();
      await waitFor(() => chunks.length === 2, 'current frame after drain');
      assert.match(chunks[1], /latest-draft/);
      assert.doesNotMatch(chunks[1], /stale-/);
      feed('\\u0015/exit\\r');
      await running;
      assert.equal(output.listenerCount('drain'), 0);
      const count = chunks.length;
      callbacks.shift()();
      await delay(30);
      assert.equal(chunks.length, count, 'frame written after terminal release');
    } finally {
      shutdown.abort();
      await running;
      output.destroy();
      Object.defineProperty(process, 'stdout', original);
    }
  `;
  for (const monochrome of [false, true]) {
    const env: NodeJS.ProcessEnv = { ...process.env, TERM: "xterm-256color" };
    if (monochrome) env["NO_COLOR"] = "1";
    else delete env["NO_COLOR"];
    const result = spawnSync(process.execPath, ["--input-type=module", "--eval", source], {
      encoding: "utf8", timeout: 5_000, windowsHide: true, env,
    });
    assert.equal(result.status, 0, result.stderr);
  }
});
