import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { parseOptions } from "../dev/tui/options.ts";
import { SCENES } from "../dev/tui/registry.ts";

const root = fileURLToPath(new URL("..", import.meta.url));

test("lab options reject invalid scenes, dimensions, time and ambiguous modes", () => {
  for (const args of [
    ["--scene", "missing"], ["--scene"], ["--size", "0x14"], ["--size", "301x14"],
    ["--size", "100x201"], ["--size", "no"], ["--time", "-1"], ["--time", "3600001"],
    ["--time", "1.2"], ["--color", "always"], ["--render", "--list"], ["--unknown"],
  ]) assert.throws(() => parseOptions(args), Error, args.join(" "));
  assert.deepEqual(parseOptions(["--scene", "tools-stream", "--size", "38x14", "--time", "123",
    "--color", "off", "--paused", "--reduced-motion", "--render"]), {
    mode: "render", scene: "tools-stream", size: { cols: 38, rows: 14 }, time: 123,
    color: "off", paused: true, reducedMotion: true,
  });
});

test("headless CLI lists the registry and renders without terminal lifecycle escapes", () => {
  const run = (...args: string[]) => spawnSync(process.execPath, ["dev/tui/main.ts", ...args], {
    cwd: root, encoding: "utf8", timeout: 10_000, env: { ...process.env, NO_COLOR: "1" },
  });
  const list = run("--list");
  assert.equal(list.status, 0, list.stderr);
  assert.deepEqual(list.stdout.trim().split(/\r?\n/).map((line) => line.split("\t")[0]), SCENES);
  const help = run("--help");
  assert.equal(help.status, 0, help.stderr);
  assert.match(help.stdout, /Ctrl\+G/);
  const frame = run("--render", "--scene", "tools-lifecycle", "--time", "4000", "--size", "100x30");
  assert.equal(frame.status, 0, frame.stderr);
  assert.equal(frame.stdout.trimEnd().split(/\r?\n/).length, 30);
  assert.match(frame.stdout, /exit 0 · 2\.4s/);
  assert.doesNotMatch(frame.stdout, /\x1b|CATALOGUE|PREVIEW/);
  const workflow = run("--render", "--scene", "tools-workflow", "--time", "33200", "--size", "100x30");
  assert.equal(workflow.status, 0, workflow.stderr);
  assert.match(workflow.stdout, /61 passed, 0 failed/);
  assert.match(workflow.stdout, /full project check has not\s+been run/);
  assert.doesNotMatch(workflow.stdout, /\x1b|esc to interrupt|CATALOGUE|PREVIEW/);
  const invalid = run("--scene", "missing");
  assert.equal(invalid.status, 1);
  assert.match(invalid.stderr, /unknown lab scene/);
  assert.equal(invalid.stdout, "");
  const noTty = run();
  assert.equal(noTty.status, 1);
  assert.match(noTty.stderr, /needs a terminal/);
  assert.doesNotMatch(noTty.stdout, /\x1b/);
});

test("all scenes stay bounded with colour, NO_COLOR, reduced motion and deterministic time", () => {
  const script = `
    import assert from "node:assert/strict";
    Object.defineProperty(process.stdout, "isTTY", { value: true });
    const { createLab } = await import("./dev/tui/controller.ts");
    const { SCENES } = await import("./dev/tui/registry.ts");
    const { WORKFLOW_MOMENTS } = await import("./dev/tui/scenarios/workflow.ts");
    const { STEEL } = await import("./src/ui/theme.ts");
    const { textWidth } = await import("./src/ui/width.ts");
    const ansi = /\\x1b\\[[0-9;]*m/g;
    let coloured = false;
    for (const scene of SCENES) for (const reducedMotion of [false, true]) {
      const lab = createLab({ scene, reducedMotion, palette: STEEL, tick: 0, selected: 0, expanded: true });
      const times = [0, 1200, 4800, ...(scene === "tools-workflow" ? WORKFLOW_MOMENTS.map(({ time }) => time) : [])];
      let previous = 0;
      for (const time of times) {
        lab.advance(time - previous);
        previous = time;
        for (const size of [{ cols: 38, rows: 14 }, { cols: 100, rows: 30 }, { cols: 160, rows: 40 }]) {
          const frame = lab.render(size);
          assert.equal(frame.rows.length, size.rows, scene);
          for (const line of frame.rows) {
            if (line.includes("\\x1b")) coloured = true;
            assert.ok(textWidth(line.replace(ansi, "")) <= size.cols, scene + ": " + line);
          }
          if (reducedMotion) assert.equal(frame.transcriptAnimating, false, scene);
        }
      }
      lab.close();
    }
    assert.equal(coloured, process.env.NO_COLOR === undefined);

    // Changing colour must invalidate transcript rows as well as host controls.
    const { runLab } = await import("./dev/tui/host.ts");
    const { parseOptions } = await import("./dev/tui/options.ts");
    let feed; let rows;
    const running = runLab(parseOptions(["--scene", "conversation", "--paused"]), {
      screen: { size: () => ({ cols: 100, rows: 30 }), enter() {}, leave() {}, setReducedMotion() {},
        onResize: () => () => {}, onInput(handler) { feed = handler; return () => {}; } },
      paint: { paint(frame) { rows = frame; }, invalidate() {} },
    });
    feed("c");
    assert.ok(rows.every(line => !line.includes("\\x1b")));
    feed("q");
    await running;
  `;
  for (const noColor of [false, true]) {
    const env: NodeJS.ProcessEnv = { ...process.env, TERM: "xterm-256color" };
    delete env["FORCE_COLOR"];
    if (noColor) env["NO_COLOR"] = "1";
    else delete env["NO_COLOR"];
    const result = spawnSync(process.execPath, ["--input-type=module", "--eval", script], {
      cwd: root, env, encoding: "utf8", timeout: 15_000,
    });
    assert.equal(result.status, 0, result.stderr);
  }
});
