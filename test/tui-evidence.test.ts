import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { TICK_MS } from "../dev/tui/model.ts";
import { workflowScene, WORKFLOW_DURATION_MS } from "../dev/tui/scenarios/workflow.ts";
import { WORKFLOW_EVIDENCE } from "../dev/tui/fixtures/workflow.ts";
import { callOf, stage, strip } from "../dev/test-support/tui.ts";
import type { Detail, ToolBlock } from "../src/tui/blocks.ts";
import { renderTool } from "../src/tui/components/tool.ts";
import { renderDetail, toolEvidence } from "../src/tui/components/tool-evidence.ts";
import { STEEL } from "../src/ui/theme.ts";
import { textWidth } from "../src/ui/width.ts";

const plain = (rows: readonly string[]) => strip(rows).join("\n");

function tool(name: string, body: Detail[], right = "applied"): ToolBlock {
  return { kind: "tool", name, target: "fixture.ts", right, tone: "ok", body, expanded: false };
}

function changes(regions = 5): Detail[] {
  return Array.from({ length: regions }, (_, index): Detail[] => [
    { kind: "del", text: `OLD_${index}_a`, oldLine: index * 10 + 1 },
    { kind: "del", text: `OLD_${index}_b`, oldLine: index * 10 + 2 },
    { kind: "add", text: `NEW_${index}_a`, newLine: index * 10 + 101 },
    { kind: "add", text: `NEW_${index}_b`, newLine: index * 10 + 102 },
    ...Array.from({ length: 3 }, (_, row): Detail => ({ kind: "keep", text: `context_${index}_${row}`,
      oldLine: index * 10 + 3 + row, newLine: index * 10 + 103 + row })),
  ]).flat();
}

function shown(block: ToolBlock, width = 100): string[] {
  const before = structuredClone(block);
  const rows = renderTool(block, width, STEEL, { now: WORKFLOW_DURATION_MS, reducedMotion: true });
  assert.ok(strip(rows).every((row) => textWidth(row) <= width));
  assert.deepEqual(block, before, "rendering preserves the complete semantic evidence");
  return rows;
}

test("large workflow evidence expands to every retained detail in source order", () => {
  const view = workflowScene({ palette: STEEL, selected: 0, expanded: false,
    scene: "tools-workflow", tick: WORKFLOW_DURATION_MS / TICK_MS });
  const tools = view.blocks.filter((block): block is ToolBlock => block.kind === "tool" && (block.body?.length ?? 0) > 0);
  assert.ok(tools.some((block) => block.name === "run_command" && block.body!.length > 40));
  assert.ok(tools.some((block) => block.name === "edit_file" &&
    block.body!.filter((detail) => detail.kind === "add" || detail.kind === "del").length >= 80));
  assert.ok(tools.some((block) => block.name === "write_file" && block.body!.length > 100));
  assert.ok(tools.some((block) => block.name === "run_command" && block.tone === "ok"));
  for (const block of tools) for (const width of [38, 100]) {
    const full = { ...block, expanded: true };
    assert.deepEqual(toolEvidence(full).details, block.body);
    const expected = block.body!.map((detail) => renderDetail(detail, block.tone, width, STEEL));
    const rows = shown(full, width);
    const start = rows.findIndex((line) => line === expected[0]);
    assert.ok(start >= 0, `${block.name} retains its first evidence row`);
    assert.deepEqual(rows.slice(start, start + expected.length), expected);
    assert.match(plain(rows), /ctrl\+o collapse/);
  }
});

test("collapsed changes retain both ends with exact counts and no duplicate outcome prefix", () => {
  const block = tool("edit_file", changes(), "+10 −10 · applied");
  const text = plain(shown(block));
  assert.equal((text.match(/\+10 −10/g) ?? []).length, 1);
  assert.match(text, /\+10 −10 · 5 regions/);
  assert.match(text, /edit_file\s+✓ applied/);
  assert.deepEqual(text.match(/(?:OLD|NEW)_\d_[ab]/g),
    ["OLD_0_a", "OLD_0_b", "NEW_0_a", "OLD_4_b", "NEW_4_a", "NEW_4_b"]);
  assert.match(text, /14 more changed lines/);
  assert.match(text, /ctrl\+o full source/);
});

test("nearby changes share a region while an omitted run starts a new region", () => {
  const add = (text: string): Detail => ({ kind: "add", text });
  const keep: Detail = { kind: "keep", text: "context" };
  for (const [between, expected] of [[[], 1], [[keep, keep], 1], [[keep, keep, keep], 2],
    [[{ kind: "gap", text: "… 100 unchanged" }], 2]] as const) {
    const block = tool("edit_file", [add("first"), ...between, add("last")]);
    assert.match(plain(shown(block)), new RegExp(`\\+2 −0 · ${expected} region`));
    assert.deepEqual(toolEvidence(block).details, [block.body![0], block.body!.at(-1)]);
  }
});

test("failed output retains a diagnostic anchor and the latest three lines", () => {
  const output: Detail[] = [
    { kind: "out", text: "not ok 1 - generic failure" },
    { kind: "out", text: "AssertionError: initial failure" },
    ...Array.from({ length: 20 }, (_, index): Detail => ({ kind: "out", text: `OUTPUT_${index}` })),
    { kind: "out", text: "ERROR_AT_END" },
  ];
  const block = { ...tool("run_command", output, "exit 1"), tone: "fail" as const };
  const text = plain(shown(block));
  assert.deepEqual(toolEvidence(block).details, [output[1], ...output.slice(-3)]);
  assert.deepEqual(text.match(/AssertionError: initial failure|OUTPUT_\d+|ERROR_AT_END/g),
    ["AssertionError: initial failure", "OUTPUT_18", "OUTPUT_19", "ERROR_AT_END"]);
  assert.doesNotMatch(text, /generic failure/);
  assert.match(text, /exit 1/);
  assert.match(text, /19 other lines · ctrl\+o/);
});

test("native Node exceptions retain their diagnostic and tail in narrow colour and NO_COLOR frames", () => {
  const script = `
    import assert from "node:assert/strict";
    import { spawnSync } from "node:child_process";
    Object.defineProperty(process.stdout, "isTTY", { value: true });
    const { hasColor } = await import("./src/ui/render.ts");
    const { textWidth } = await import("./src/ui/width.ts");
    const { compose } = await import("./src/tui/view.ts");
    const { toolEvidence } = await import("./src/tui/components/tool-evidence.ts");
    const { base, callOf, stage, strip } = await import("./dev/test-support/tui.ts");
    assert.equal(hasColor(), process.env.NO_COLOR === undefined);
    for (const name of ["TypeError", "ReferenceError", "SyntaxError"]) {
      const crashed = spawnSync(process.execPath, ["--eval", 'throw new ' + name + '("fixture failure")'], {
        encoding: "utf8", timeout: 5_000,
      });
      assert.equal(crashed.error, undefined);
      assert.equal(crashed.status, 1);
      const { blocks, events } = stage([]);
      const call = callOf("exception", "run_command", { command: "node fixture.js" });
      events.onToolCall(call);
      events.onToolResult(call, { kind: "tool_result", id: call.id, output: crashed.stderr, isError: true }, "exit 1");
      const block = blocks[0];
      assert.equal(block.kind, "tool");
      const source = structuredClone(block.body);
      const diagnosis = block.body.find(detail => detail.text === name + ": fixture failure");
      assert.ok(diagnosis, "real Node output contains the exception diagnostic");
      assert.ok(block.body.length > 4);
      assert.deepEqual(toolEvidence(block).details, [diagnosis, ...block.body.slice(-3)]);
      assert.ok(toolEvidence(block).note.startsWith(String(block.body.length - 4) + " other lines"));
      for (const size of [{ cols: 38, rows: 14 }, { cols: 100, rows: 30 }]) {
        const frame = compose({ ...base(), blocks, now: 0, reducedMotion: true }, size);
        assert.equal(frame.rows.length, size.rows);
        assert.ok(strip(frame.rows).every(row => textWidth(row) <= size.cols));
        assert.ok(strip(frame.rows).some(row => row.includes(diagnosis.text)));
        assert.equal(frame.rows.some(row => row.includes(String.fromCharCode(27))), hasColor());
        assert.ok(frame.cursor.row >= 0 && frame.cursor.row < size.rows);
        assert.ok(frame.cursor.col >= 0 && frame.cursor.col < size.cols);
        assert.deepEqual(block.body, source);
      }
      assert.deepEqual(toolEvidence({ ...block, expanded: true }).details, source);
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
    assert.equal(result.error, undefined);
    assert.equal(result.status, 0, result.stderr);
  }
});

test("a coded Node exception takes priority over an error name echoed in source", () => {
  const output: Detail[] = [
    { kind: "out", text: "const sample = 'TypeError: source context';" },
    { kind: "out", text: "TypeError [ERR_INVALID_ARG_TYPE]: The first argument must be a string" },
    ...Array.from({ length: 8 }, (_, index): Detail => ({ kind: "out", text: `    at fixture.js:${index + 1}:3` })),
  ];
  const block = { ...tool("run_command", output, "exit 1"), tone: "fail" as const };
  assert.deepEqual(toolEvidence(block).details, [output[1], ...output.slice(-3)]);
  assert.deepEqual(toolEvidence({ ...block, expanded: true }).details, output);
  assert.match(plain(shown(block)), /TypeError \[ERR_INVALID_ARG_TYPE\]/);
  assert.doesNotMatch(plain(shown(block)), /source context/);
});

test("successful output keeps four tail rows and a diagnostic already in the tail is not duplicated", () => {
  const output: Detail[] = Array.from({ length: 12 }, (_, index) => ({ kind: "out", text: `LINE_${index}` }));
  const success = tool("run_command", output, "exit 0");
  assert.deepEqual(toolEvidence(success).details, output.slice(-4));
  assert.match(plain(shown(success)), /8 earlier lines · ctrl\+o/);
  output[10] = { kind: "out", text: "error: recent failure" };
  const failure = { ...success, tone: "fail" as const, right: "exit 1" };
  assert.deepEqual(toolEvidence(failure).details, output.slice(-4));
  assert.equal((plain(shown(failure)).match(/error: recent failure/g) ?? []).length, 1);
});

test("small inputs and denied changes do not invent omissions or successful outcomes", () => {
  for (const count of [0, 1, 4]) {
    const body: Detail[] = Array.from({ length: count }, (_, index) => ({ kind: "out", text: `LINE_${index}` }));
    const block = tool("run_command", body, "exit 0");
    assert.deepEqual(toolEvidence(block).details, body);
    const text = plain(shown(block));
    assert.doesNotMatch(text, /earlier|other lines|more changed|full source/);
    if (count > 0) assert.match(text, new RegExp(`└─ ${count} line`));
  }
  for (const [tone, right] of [["deny", "denied"], ["fail", "interrupted"]] as const) {
    const block = { ...tool("edit_file", changes(1), right), tone };
    const text = plain(shown(block));
    assert.match(text, new RegExp(right));
    assert.doesNotMatch(text, /applied|created|more changed|more regions|source folded/);
    assert.match(text, /OLD_0_a/);
    assert.match(text, /NEW_0_b/);
  }
});

test("interrupting a large production preview preserves its source through expansion", () => {
  const { blocks, events } = stage([]);
  const call = callOf("edit", "edit_file", { path: "http.ts", old_text: "before", new_text: "after" });
  events.onToolCall(call, { before: WORKFLOW_EVIDENCE.sourceBefore, after: WORKFLOW_EVIDENCE.sourceAfter });
  events.onToolStart?.(call, 1, 1);
  const block = blocks[0];
  assert.ok(block?.kind === "tool");
  const source = structuredClone(block.body);
  assert.ok((source?.length ?? 0) > 80);
  events.finish("interrupted");
  assert.equal(block.right, "interrupted");
  assert.equal(block.tone, "fail");
  for (const expanded of [false, true, false]) {
    block.expanded = expanded;
    const rows = shown(block);
    assert.match(plain(rows), /interrupted/);
    assert.deepEqual(block.body, source);
    if (expanded) assert.deepEqual(toolEvidence(block).details, source);
  }
});
