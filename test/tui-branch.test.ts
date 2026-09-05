import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { TICK_MS } from "../dev/tui/model.ts";
import { workflowScene, WORKFLOW_DURATION_MS } from "../dev/tui/scenarios/workflow.ts";
import { strip } from "../dev/test-support/tui.ts";
import { render } from "../src/tui/blocks.ts";
import { renderUser } from "../src/tui/components/messages.ts";
import * as edit from "../src/tui/editor.ts";
import { transcriptRenderer } from "../src/tui/transcript-view.ts";
import type { TranscriptRenderer } from "../src/tui/transcript-view.ts";
import { compose } from "../src/tui/view.ts";
import type { View } from "../src/tui/view.ts";
import type { Size } from "../src/tui/screen.ts";
import { STEEL } from "../src/ui/theme.ts";
import { textWidth } from "../src/ui/width.ts";

const initial = { palette: STEEL, selected: 0, expanded: false, scene: "tools-workflow", tick: 0 };

function settledFrame(view: View, size: Size, transcript: TranscriptRenderer = transcriptRenderer()) {
  for (let attempt = 0; attempt < 16; attempt++) {
    const frame = compose(view, size, transcript);
    if (!frame.transcriptPending) return frame;
  }
  throw new Error("the workflow transcript did not finish reflowing");
}

test("the response measure preserves the full-width composer and grapheme caret", () => {
  const long = Array.from({ length: 12 }, (_, index) => `${index}: 保持 👩‍💻 é ${"detail ".repeat(15)}`).join("\n");
  for (const draft of ["", long]) {
    const view = workflowScene({ ...initial, tick: WORKFLOW_DURATION_MS / TICK_MS });
    view.editor = edit.of(draft);
    const source = structuredClone(view.blocks);
    for (const size of [{ cols: 38, rows: 14 }, { cols: 68, rows: 18 }, { cols: 160, rows: 40 }]) {
      const frame = settledFrame(view, size);
      assert.equal(frame.rows.length, size.rows);
      assert.ok(strip(frame.rows).every((row) => textWidth(row) <= size.cols));
      assert.ok(frame.cursor !== undefined);
      assert.ok(frame.cursor.row >= 0 && frame.cursor.row < size.rows);
      assert.ok(frame.cursor.col >= 0 && frame.cursor.col < size.cols);
      assert.equal(view.editor.text, draft);
      const empty = compose({ ...view, blocks: [] }, size);
      const firstInk = strip(empty.rows).findIndex((line) => line.trim() !== "");
      assert.ok(firstInk > 0);
      assert.deepEqual(frame.rows.slice(firstInk - 1), empty.rows.slice(firstInk - 1));
      assert.deepEqual(frame.cursor, empty.cursor);
      assert.deepEqual(view.blocks, source);
    }
  }
});

test("the production response inset leaves the accepted user surface unchanged", () => {
  const user = { kind: "user", text: "Check `src/providers/http.ts` — 保持 👩‍💻 é\nKeep the change small." } as const;
  const before = structuredClone(user);
  for (const width of [38, 80, 160]) assert.deepEqual(render(user, width, STEEL), renderUser(user, width, STEEL));
  assert.deepEqual(user, before);
});

test("expanding source and collapsing it restores the same cached production viewport", () => {
  const view = { ...workflowScene({ ...initial, tick: WORKFLOW_DURATION_MS / TICK_MS }), reducedMotion: true };
  const block = view.blocks.find((entry) => entry.kind === "tool" && entry.name === "write_file");
  assert.ok(block?.kind === "tool");
  const source = structuredClone(block.body);
  for (const size of [{ cols: 38, rows: 14 }, { cols: 100, rows: 30 }]) {
    const transcript = transcriptRenderer();
    const baseline = settledFrame(view, size, transcript);
    block.expanded = true;
    transcript.invalidate(block);
    const expanded = settledFrame(view, size, transcript);
    assert.ok(expanded.maxScroll > baseline.maxScroll);
    assert.deepEqual(block.body, source);
    block.expanded = false;
    transcript.invalidate(block);
    assert.deepEqual(settledFrame(view, size, transcript), baseline);
  }
});

test("only running connectors animate under fresh colour initialization", () => {
  const script = `
    import assert from "node:assert/strict";
    Object.defineProperty(process.stdout, "isTTY", { value: true });
    const { renderTool } = await import("./src/tui/components/tool.ts");
    const { render } = await import("./src/tui/blocks.ts");
    const { renderUser } = await import("./src/tui/components/messages.ts");
    const { transcriptRenderer } = await import("./src/tui/transcript-view.ts");
    const { compose } = await import("./src/tui/view.ts");
    const { workflowScene, WORKFLOW_TIMES: times } = await import("./dev/tui/scenarios/workflow.ts");
    const { hasColor } = await import("./src/ui/render.ts");
    const { STEEL } = await import("./src/ui/theme.ts");
    const { textWidth } = await import("./src/ui/width.ts");
    const { TICK_MS } = await import("./dev/tui/model.ts");
    assert.equal(hasColor(), process.env.NO_COLOR === undefined);
    const at = now => workflowScene({ scene: "tools-workflow", palette: STEEL,
      selected: 0, expanded: false, tick: now / TICK_MS });
    const command = now => at(now).blocks.find(block => block.kind === "tool" && block.name === "run_command");
    const running = command(times.commandStartedMs + 1_000);
    const waiting = command(times.commandWaitingMs);
    const settled = command(times.commandSettledMs);
    assert.equal(running.right, "running");
    assert.equal(waiting.right, "waiting");
    assert.equal(settled.tone, "fail");
    const plain = rows => rows.map(row => row.replace(/\\x1b\\[[0-9;]*m/g, ""));
    const contentInk = rows => rows.map(row => {
      let foreground = "";
      const content = [];
      for (const token of row.match(/\\x1b\\[[0-9;]*m|[^\\x1b]/gu) ?? []) {
        if (token.startsWith("\\x1b")) {
          if (token.startsWith("\\x1b[38;2;")) foreground = token;
          else if (token === "\\x1b[0m" || token === "\\x1b[39m") foreground = "";
        } else if (!/[\\s\\u2500-\\u257f]/u.test(token)) content.push([token, foreground]);
      }
      return content;
    });
    // Freeze the evidence and displayed duration while sampling decorative activity.
    const first = times.commandStartedMs + 10_050;
    const second = times.commandStartedMs + 10_400;
    const draw = (block, now, reducedMotion = false) => renderTool(block, 100, STEEL, { now, reducedMotion });
    const before = draw(running, first);
    const samples = Array.from({ length: 6 }, (_, index) => draw(running, first + index * 70));
    if (hasColor()) assert.ok(samples.some(rows => JSON.stringify(rows) !== JSON.stringify(before)));
    else samples.forEach(rows => assert.deepEqual(rows, before, "NO_COLOR rests"));
    samples.forEach(rows => assert.deepEqual(plain(rows), plain(before), "glyphs and geometry stay fixed"));
    samples.forEach(rows => assert.deepEqual(contentInk(rows), contentInk(before), "only the gutter changes colour"));
    assert.deepEqual(draw(running, first, true), draw(running, second, true));
    assert.deepEqual(draw(waiting, first), draw(waiting, second));
    assert.deepEqual(draw(settled, first), draw(settled, second));
    const expanded = { ...running, expanded: true };
    assert.deepEqual(draw(expanded, first), draw(expanded, second), "expanded source rests");
    const user = at(0).blocks[0];
    const expected = renderUser(user, 80, STEEL);
    assert.equal(expected.some(row => row.includes("\\x1b[48;")), hasColor());
    assert.deepEqual(render(user, 80, STEEL), expected);
    for (const reducedMotion of [false, true]) {
      const transcript = transcriptRenderer();
      const blocks = [running];
      transcript.invalidate(running);
      const first = transcript.viewport(blocks, 100, 20, 0, STEEL,
        { now: times.commandStartedMs + 1_000, reducedMotion });
      const later = transcript.viewport(blocks, 100, 20, 0, STEEL,
        { now: times.commandStartedMs + 2_000, reducedMotion });
      assert.equal(first.animating, hasColor() && !reducedMotion);
      assert.equal(later.animating, first.animating);
      assert.match(plain(first.rows).join("\\n"), /running · 1\\.0s/);
      assert.match(plain(later.rows).join("\\n"), /running · 2\\.0s/);
    }
    for (const block of [waiting, settled, expanded]) {
      const transcript = transcriptRenderer();
      transcript.invalidate(block);
      assert.equal(transcript.viewport([block], 100, 20, 0, STEEL, { now: first }).animating, false);
    }
    const history = [running, ...Array.from({ length: 20 }, (_, index) => ({ kind: "answer", text: "answer " + index }))];
    const transcript = transcriptRenderer();
    transcript.invalidate(running);
    const tail = transcript.viewport(history, 100, 8, 0, STEEL, { now: first });
    assert.equal(tail.animating, false, "an offscreen running tool does not schedule animation");
    const reading = transcript.viewport(history, 100, 8, tail.maxScroll, STEEL, { now: first });
    assert.equal(reading.animating, false);
    assert.deepEqual(transcript.viewport(history, 100, 8, tail.maxScroll, STEEL, { now: second }), reading);
    for (const [now, name, minimum] of [
      [times.editStartedMs + 800, "edit_file", 80], [times.writeStartedMs + 1_200, "write_file", 100],
    ]) {
      const view = at(now);
      const active = view.blocks.findLast(block => block.kind === "tool");
      assert.equal(active.name, name);
      assert.equal(active.right, "running");
      assert.ok(active.body.length >= minimum);
      for (const size of [{ cols: 38, rows: 14 }, { cols: 100, rows: 30 }]) {
        const cache = transcriptRenderer();
        let frame = compose(view, size, cache);
        for (let attempt = 0; frame.transcriptPending && attempt < 16; attempt++) frame = compose(view, size, cache);
        assert.equal(frame.transcriptPending, false);
        assert.equal(frame.rows.length, size.rows);
        assert.ok(plain(frame.rows).every(row => textWidth(row) <= size.cols));
        assert.ok(frame.cursor.row >= 0 && frame.cursor.row < size.rows);
        assert.ok(frame.cursor.col >= 0 && frame.cursor.col < size.cols);
      }
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
