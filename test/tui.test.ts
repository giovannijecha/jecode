import { test } from "node:test";
import assert from "node:assert/strict";
import * as edit from "../src/tui/editor.ts";
import { compose } from "../src/tui/view.ts";
import { COMMANDS } from "../src/commands.ts";
import { textWidth } from "../src/ui/width.ts";
import { row } from "../src/ui/render.ts";
import type { Block } from "../src/tui/blocks.ts";
import { renderAll } from "../src/tui/blocks.ts";
import { transcribe } from "../src/tui/turn.ts";
import * as approve from "../src/tui/approve.ts";
import * as picker from "../src/tui/picker.ts";
import * as field from "../src/tui/field.ts";
import { STEEL } from "../src/ui/theme.ts";
import {
  activate as activateCompletion,
  matches,
  move as moveCompletion,
  options as completionOptions,
  pick,
  selected as selectedCompletion,
} from "../src/tui/complete.ts";

test("a frame is exactly as tall as the terminal", () => {
  for (const rows of [10, 24, 60]) {
    const frame = compose(base(), { rows, cols: 80 });
    assert.equal(frame.rows.length, rows);
  }
});

test("a right column that fills the row never adds an extra gutter cell", () => {
  const rendered = row(5, [{ text: "left" }], [{ text: "12345" }]);
  assert.equal(textWidth(rendered), 5);
});

test("the cursor starts on the same column as the transcript", () => {
  const frame = compose(base(), { rows: 24, cols: 80 });
  // Nineteen transcript rows, one rhythm row, then the composer border.
  assert.deepEqual(frame.cursor, { row: 21, col: 0 });
});

test("the cursor follows the text as the input wraps", () => {
  const text = "x".repeat(100);
  const frame = compose({ ...base(), editor: edit.of(text) }, { rows: 24, cols: 40 });
  // 100 chars over a 40-wide dock is three rows; the cursor is on the last.
  assert.deepEqual(frame.cursor, { row: 21, col: 20 });
});

test("scroll is bounded by how much there is to scroll", () => {
  const view = {
    ...base(),
    blocks: Array.from({ length: 40 }, (_, i) => ({ kind: "answer" as const, text: `line ${i}` })),
    scroll: 9999,
  };
  const frame = compose(view, { rows: 24, cols: 80 });
  // Each assistant block owns a spacer, against a nineteen-row viewport.
  assert.equal(frame.maxScroll, 40 * 2 - (24 - 5));
  assert.match(frame.rows.slice(0, 2).join("\n"), /line 0/);
});

test("the one-line footer carries model, effort, and workspace without provider noise", () => {
  const frame = compose(base(), { rows: 24, cols: 80 });
  const footer = strip(frame.rows)[23] as string;
  assert.match(footer, /claude-sonnet-5 · high · ~\/Codex\/jecode/);
  assert.doesNotMatch(footer, /anthropic|cloud|ready|tokens/);
});

test("the footer exposes unseen output while scroll lock is active", () => {
  const frame = compose({ ...base(), unseen: 3 }, { rows: 24, cols: 80 });
  assert.match(strip(frame.rows).join("\n"), /3 new ↓/);
});

test("activity leaves only the interrupt hint on the footer's right edge", () => {
  const quiet = compose(base(), { rows: 24, cols: 80 });
  const active = compose(
    { ...base(), status: "Writing · 2s" },
    { rows: 24, cols: 80 },
  );
  const blocked = compose(
    {
      ...base(),
      readiness: { text: "Anthropic needs an API key · /settings", tone: "warn" },
    },
    { rows: 24, cols: 80 },
  );
  const activeRows = strip(active.rows);
  const blockedRows = strip(blocked.rows);

  assert.deepEqual(active.cursor, quiet.cursor);
  assert.equal(activeRows.findIndex((line) => line.includes("esc to interrupt")), 23);
  assert.equal(activeRows.findIndex((line) => line.includes("Writing · 2s")), -1);
  assert.equal(blockedRows.findIndex((line) => line.includes("needs an API key")), 23);
  assert.match(activeRows[23] ?? "", /claude-sonnet-5 · high/);
});

test("urgent feedback outranks activity while informational feedback waits behind it", () => {
  const urgent = strip(compose({
    ...base(),
    status: "Writing · 2s",
    feedback: { text: "request failed", tone: "error" },
  }, { rows: 24, cols: 80 }).rows)[23] ?? "";
  const informational = strip(compose({
    ...base(),
    status: "Writing · 2s",
    feedback: { text: "settings saved", tone: "info" },
  }, { rows: 24, cols: 80 }).rows)[23] ?? "";

  assert.match(urgent, /× request failed/);
  assert.doesNotMatch(urgent, /Writing/);
  assert.match(informational, /esc to interrupt/);
  assert.doesNotMatch(informational, /Writing/);
  assert.doesNotMatch(informational, /settings saved/);
});

test("every frame matches the real terminal height, including small modals", () => {
  const prompt = {
    title: [],
    options: Array.from({ length: 20 }, (_, index) => ({ label: `model-${index}` })),
    searchable: true,
    query: "",
    index: 0,
  };
  for (const rows of [8, 10, 12]) {
    const frame = compose({ ...base(), modal: { kind: "pick" as const, picker: prompt } }, { rows, cols: 50 });
    assert.equal(frame.rows.length, rows);
  }
});

test("transient feedback stays visible when a command reopens its parent picker", () => {
  const modal = {
    kind: "pick" as const,
    picker: { title: [], options: [{ label: "credentials" }], index: 0 },
  };
  const frame = compose(
    {
      ...base(),
      status: "Running /settings · 1s",
      feedback: { text: "credential saved · ~/.jecode/credentials.json", tone: "info" as const },
      modal,
    },
    { rows: 24, cols: 80 },
  );
  const rows = strip(frame.rows);
  const feedback = rows.findIndex((line) => line.includes("credential saved"));
  const option = rows.findIndex((line) => line.includes("→ credentials"));
  assert.equal(feedback, rows.length - 1);
  assert.ok(option < feedback);
  assert.equal(rows.length, 24);

  const quiet = strip(compose({ ...base(), modal }, { rows: 24, cols: 80 }).rows);
  assert.equal(quiet.findIndex((line) => line.includes("→ credentials")), option);
});

test("a genuinely tiny terminal gets an exact-height recovery screen", () => {
  const frame = compose(base(), { rows: 4, cols: 20 });
  assert.equal(frame.rows.length, 4);
  assert.match(strip(frame.rows).join("\n"), /too small/);
  assert.equal(frame.cursor, undefined);
});

function base() {
  return {
    blocks: [],
    editor: edit.EMPTY,
    scroll: 0,
    pal: STEEL,
    footer: {
      workspace: "~/Codex/jecode",
      model: "claude-sonnet-5",
      effort: "high",
    },
    spin: 0,
  };
}

test("backspace removes a whole emoji, not half of one", () => {
  const state = edit.backspace(edit.of("ciao 🙂"));
  assert.equal(state.text, "ciao ");
});

test("the cursor counts cells, so a wide glyph moves it two columns", () => {
  const frame = compose({ ...base(), editor: edit.of("日本") }, { rows: 24, cols: 80 });
  assert.equal(frame.cursor?.col, 4);
});

test("a half-typed command offers the ones it could still be", () => {
  assert.deepEqual(
    matches("/he").map((command) => command.name),
    ["help"],
  );
  assert.equal(matches("/help me").length, 0);
  assert.deepEqual(
    matches("/eff").map((command) => command.name),
    ["effort"],
  );
});

test("completion selection wraps without rewriting the typed prefix", () => {
  assert.equal(pick("/", 0), "/help");
  assert.equal(pick("/", COMMANDS.length), "/help");
  assert.equal(pick("/nonexistent", 0), undefined);

  const completion = activateCompletion("/");
  assert.ok(completion !== undefined);
  const next = moveCompletion(completion, 1);
  assert.equal(next.prefix, "/");
  assert.equal(selectedCompletion(next), "/exit");
  assert.equal(selectedCompletion(moveCompletion(completion, -1)), "/providers");
});

test("closing completion hides suggestions even when the editor still holds a command", () => {
  assert.deepEqual(completionOptions(undefined), []);
  assert.deepEqual(
    completionOptions(activateCompletion("/exit")).map((command) => command.name),
    ["exit"],
  );
});

test("the command menu follows an arrow selection beyond its first window", () => {
  const frame = compose(
    { ...base(), editor: edit.of("/"), menu: COMMANDS, menuIndex: 5 },
    { rows: 24, cols: 80 },
  );
  const shown = strip(frame.rows).join("\n");
  assert.match(shown, /→ \/settings/);
  assert.doesNotMatch(shown, /\/help\s+show keyboard controls/);
});

test("command suggestions stay between the composer's two rails", () => {
  const frame = compose(
    { ...base(), editor: edit.of("/"), menu: COMMANDS, menuIndex: 0 },
    { rows: 24, cols: 80 },
  );
  const rows = strip(frame.rows);
  const input = frame.cursor?.row ?? -1;
  const first = rows.findIndex((line) => line.includes("/help"));
  const last = rows.findIndex((line) => line.includes("/export"));
  assert.equal(rows[input - 1], "─".repeat(80));
  assert.match(rows[input] ?? "", /^\/.*1–4 \/ 11$/);
  assert.equal(first, input + 1);
  assert.equal(rows[last + 1], "─".repeat(80));
  assert.match(rows[first] ?? "", /→/);
});

test("a searchable picker filters without losing the original option index", () => {
  const source: picker.Picker = {
    title: [],
    options: [{ label: "alpha" }, { label: "beta" }, { label: "betamax" }, { label: "gamma" }],
    searchable: true,
    query: "",
    index: 0,
  };
  const filtered = picker.type(source, "beta");
  assert.equal(filtered.index, 1);
  assert.equal(picker.move(filtered, 1).index, 2);
  assert.equal(picker.edge(filtered, "end").index, 2);
  const rendered = strip(picker.panel(filtered, 50, STEEL)).join("\n");
  assert.match(rendered, /→ beta.*1–2 \/ 2 · 4 total/);
  assert.deepEqual(picker.caret(filtered, 50), { row: 1, col: 6 });
  assert.equal(picker.clear(filtered).query, "");
  const empty = picker.type(source, "missing");
  assert.equal(picker.selected(empty), undefined);
});

test("reduced motion replaces the live rail animation with a stable node", () => {
  const frame = compose({
    ...base(),
    blocks: [{
      kind: "tool",
      name: "run_command",
      target: "npm test",
      right: "running",
      tone: "pending",
      startedAt: 0,
    }],
    status: "Running run_command",
    reducedMotion: true,
    now: 1_000,
  }, { rows: 24, cols: 80 });
  const shown = strip(frame.rows).join("\n");
  assert.match(shown, /◌ run_command\s+npm test/);
  assert.match(shown, /running · 1\.0s/);
  assert.match(shown, /esc to interrupt/);
});

function stage(log: string[]) {
  const blocks: Block[] = [];
  const events = transcribe({
    emit: (block) => blocks.push(block),
    render: () => {},
    ask: (_prompt, settle) => settle("once"),
    approved: () => true,
    remember: () => {},
    status: (text) => log.push(text),
    palette: STEEL,
  });
  return { blocks, events };
}

const callOf = (id: string, name: string, input: Record<string, unknown>) =>
  ({ kind: "tool_call", id, name, input }) as const;

test("transcription tracks the live activity phase", () => {
  const log: string[] = [];
  const { events } = stage(log);
  const call = callOf("1", "run_command", { command: "node run.js" });

  events.onStream({ kind: "thinking", text: "hm" });
  events.onStream({ kind: "text", text: "ok" });
  events.onToolCall(call);
  events.onToolResult(call, { kind: "tool_result", id: "1", output: "11", isError: false }, "exit 0");

  assert.deepEqual(log, ["Thinking", "Writing", "Running run_command", "Waiting"]);
});

test("an edit is diffed from the call, so approval is about the change", () => {
  const { blocks, events } = stage([]);
  events.onToolCall(
    callOf("1", "edit_file", { path: "a.js", old_text: "let x = 1;", new_text: "const x = 1;" }),
  );

  const [block] = blocks;
  assert.equal(block?.kind, "tool");
  assert.deepEqual(
    block?.kind === "tool" ? block.body?.map((d) => `${d.kind}:${d.text}`) : undefined,
    ["del:let x = 1;", "add:const x = 1;"],
  );
  if (block?.kind !== "tool") return;
  assert.deepEqual(block.body?.[0], {
    kind: "del",
    text: "let x = 1;",
    oldLine: 1,
    emphasis: { start: 0, length: 3 },
  });
  assert.deepEqual(block.body?.[1], {
    kind: "add",
    text: "const x = 1;",
    newLine: 1,
    emphasis: { start: 0, length: 5 },
  });
  const rendered = strip(renderAll([block], 60, STEEL)).join("\n");
  assert.match(rendered, /-\s+1 let x = 1;/);
  assert.match(rendered, /\+\s+1 const x = 1;/);
});

test("a tool is a compact execution rail with evidence beneath it", () => {
  const block: Block = {
    kind: "tool",
    name: "run_command",
    target: "ls",
    right: "exit 0",
    tone: "ok",
    body: [
      { kind: "out", text: "one" },
      { kind: "out", text: "two" },
    ],
  };

  const ESCAPE = String.fromCharCode(27);
  const bare = renderAll([block], 60, STEEL).map((r) => r.replace(new RegExp(`${ESCAPE}\[[0-9;]*m`, "g"), ""));
  assert.match(bare.join("\n"), /✓ run_command\s+ls/);
  assert.match(bare.join("\n"), /│ one/);
  assert.match(bare.join("\n"), /│ two/);
  assert.equal(bare[0], "");
  assert.ok(bare.every((line) => textWidth(line) <= 60));
  assert.ok(bare.slice(1).every((line) => !/[ \t]+$/.test(line)));
});

test("an approval is a menu of answers, not a key to guess at", () => {
  const prompt = approve.promptFor(callOf("1", "write_file", { path: "package.json" }), "package.json", STEEL);
  const bare = strip(picker.panel(prompt, 80, STEEL));
  const shown = bare.join("\n");

  assert.match(shown, /Write this file\?.*write_file · package\.json/);
  assert.match(shown, /→ Yes, once/);
  assert.match(shown, /Yes, this file for the session/);
  assert.match(shown, /No, and say why/);
  assert.doesNotMatch(shown, /Enter to select|Permission required/);
});

test("the arrows walk the answers and wrap rather than dead-ending", () => {
  const prompt = approve.promptFor(callOf("1", "run_command", { command: "ls" }), "ls", STEEL);
  assert.equal(approve.answerAt(picker.move(prompt, 1).index), "always");
  assert.equal(approve.answerAt(picker.move(prompt, -1).index), "no");
  assert.equal(approve.answerAt(picker.move(picker.move(prompt, 1), -1).index), "once");
});

test("a key that answers nothing neither approves nor refuses", () => {
  const prompt = approve.promptFor(callOf("1", "write_file", { path: "a.js" }), "a.js", STEEL);
  assert.equal(picker.byKey(prompt, "q"), undefined);
  assert.equal(approve.answerAt(picker.byKey(prompt, "y")), "once");
  assert.equal(approve.answerAt(picker.byKey(prompt, "2")), "always");
  assert.equal(picker.byKey(prompt, "9"), undefined);
});

test("answering always stops the question for the same file, not every file", async () => {
  const allowed = new Set<string>();
  const asked: string[] = [];
  const events = transcribe({
    emit: () => {},
    render: () => {},
    ask: (prompt, settle) => {
      asked.push(prompt.options.length === 3 ? "write_file" : "?");
      settle("always");
    },
    approved: (call) => allowed.has(approve.scopeFor(call).key),
    remember: (call) => allowed.add(approve.scopeFor(call).key),
    status: () => {},
    palette: STEEL,
  });

  const call = callOf("1", "write_file", { path: "a.js" });
  assert.equal(await events.approve(call), true);
  assert.equal(await events.approve(callOf("2", "edit_file", { path: "a.js" })), true);
  assert.equal(await events.approve(callOf("3", "write_file", { path: "b.js" })), true);
  assert.deepEqual(asked, ["write_file", "write_file"]);
});

test("reasoning keeps an unframed three-row tail and expands without losing text", () => {
  const block: Block = {
    kind: "reasoning",
    text: "- line one\n- line two\n- line three\n- line four",
    live: true,
    expanded: false,
  };
  const preview = strip(renderAll([block], 50, STEEL));
  const shown = preview.join("\n");
  assert.match(shown, /thinking/);
  assert.doesNotMatch(shown, /· live|thought/);
  assert.doesNotMatch(shown, /line one/);
  assert.match(shown, /line two/);
  assert.match(shown, /line three/);
  assert.match(shown, /line four/);
  assert.match(shown, /ctrl\+o full/);
  assert.equal(preview.length, 5, "outer gap, heading, and three content rows");
  assert.equal(preview[0], "");
  assert.ok(preview.slice(2).every((line) => line !== " ".repeat(50)));
  assert.doesNotMatch(shown, /─/);

  block.live = false;
  block.expanded = true;
  const full = strip(renderAll([block], 50, STEEL)).join("\n");
  assert.match(full, /thought/);
  assert.doesNotMatch(full, /thinking|· live/);
  assert.match(full, /line one/);
  assert.match(full, /ctrl\+o compact/);
});

test("reasoning changes from live to retained preview at the next stream kind", () => {
  const { blocks, events } = stage([]);
  events.onStream({ kind: "thinking", text: "inspect" });
  const reasoning = blocks[0];
  assert.equal(reasoning?.kind, "reasoning");
  if (reasoning?.kind !== "reasoning") return;
  assert.equal(reasoning.live, true);
  assert.equal(reasoning.expanded, false);

  events.onStream({ kind: "text", text: "done" });
  assert.equal(reasoning.live, false);
});

test("a terminal escape split across stream chunks remains inert", () => {
  const { blocks, events } = stage([]);
  const escape = String.fromCharCode(27);
  const bell = String.fromCharCode(7);
  events.onStream({ kind: "text", text: escape });
  events.onStream({ kind: "text", text: `]52;c;payload${bell}` });

  const drawn = strip(renderAll(blocks, 60, STEEL)).join("\n");
  assert.equal(drawn.includes(escape), false);
  assert.ok(drawn.includes(`${String.fromCodePoint(0x241b)}]52`));
});

test("a reasoning-only stream is sealed when transcription finishes", () => {
  const { blocks, events } = stage([]);
  events.onStream({ kind: "thinking", text: "inspect" });
  events.finish();
  const reasoning = blocks[0];
  assert.equal(reasoning?.kind === "reasoning" ? reasoning.live : undefined, false);
});

test("finishing an interrupted transcription settles its running tool", () => {
  const { blocks, events } = stage([]);
  events.onToolCall(callOf("1", "run_command", { command: "npm test" }));

  events.finish("interrupted");

  const tool = blocks[0];
  assert.equal(tool?.kind === "tool" ? tool.tone : undefined, "fail");
  assert.equal(tool?.kind === "tool" ? tool.right : undefined, "interrupted");
  assert.equal(tool?.kind === "tool" ? tool.startedAt : undefined, undefined);
});

function strip(rows: readonly string[]): string[] {
  const ESCAPE = String.fromCharCode(27);
  return rows.map((r) => r.replace(new RegExp(`${ESCAPE}\[[0-9;]*m`, "g"), ""));
}

test("a write over an existing file is shown as the change, not as a new file", () => {
  const { blocks, events } = stage([]);
  events.onToolCall(callOf("1", "write_file", { path: "conf.json", content: "a\nCHANGED\nc\n" }), {
    before: "a\nb\nc\n",
    after: "a\nCHANGED\nc\n",
  });

  const [block] = blocks;
  assert.deepEqual(
    block?.kind === "tool" ? block.body?.map((d) => `${d.kind}:${d.text}`) : undefined,
    ["keep:a", "del:b", "add:CHANGED", "keep:c"],
  );
});

test("tool output is retained in full and only collapsed while rendering", () => {
  const { blocks, events } = stage([]);
  const call = callOf("1", "run_command", { command: "many" });
  events.onToolCall(call);
  events.onToolResult(
    call,
    { kind: "tool_result", id: "1", output: Array.from({ length: 20 }, (_, index) => `line ${index}`).join("\n"), isError: false },
    "exit 0",
  );
  const block = blocks[0];
  assert.equal(block?.kind, "tool");
  if (block?.kind !== "tool") return;
  assert.equal(block.body?.length, 20);
  assert.match(strip(renderAll([block], 60, STEEL)).join("\n"), /ctrl\+o expand/);
  block.expanded = true;
  const expanded = strip(renderAll([block], 60, STEEL)).join("\n");
  assert.match(expanded, /line 19/);
  assert.doesNotMatch(expanded, /ctrl\+o expand/);
});

test("live command output updates the existing pending rail", () => {
  const { blocks, events } = stage([]);
  const call = callOf("1", "run_command", { command: "many" });
  events.onToolCall(call);
  events.onToolOutput?.(call, Array.from({ length: 12 }, (_, index) => `line ${index}`).join("\n"));

  const block = blocks[0];
  assert.equal(block?.kind, "tool");
  if (block?.kind !== "tool") return;
  assert.equal(block.body?.length, 12);
  const shown = strip(renderAll([block], 60, STEEL, { now: block.startedAt })).join("\n");
  assert.match(shown, /12 lines so far/);
  assert.match(shown, /line 11/);
  assert.doesNotMatch(shown, /line 0/);
});

test("denying a preview keeps the evidence on its rail", async () => {
  const blocks: Block[] = [];
  const events = transcribe({
    emit: (block) => blocks.push(block),
    render: () => {},
    ask: (_prompt, settle) => settle("no"),
    approved: () => false,
    remember: () => {},
    status: () => {},
    palette: STEEL,
  });
  const call = callOf("1", "edit_file", { path: "a.ts", old_text: "a", new_text: "b" });
  events.onToolCall(call);
  assert.equal(await events.approve(call), false);
  events.onToolResult(call, { kind: "tool_result", id: "1", output: "declined", isError: true }, "declined");

  const block = blocks[0];
  assert.equal(block?.kind === "tool" ? block.tone : undefined, "deny");
  assert.equal(block?.kind === "tool" ? block.right : undefined, "denied");
  assert.equal(block?.kind === "tool" ? block.body?.length : undefined, 2);
});

test("conversation blocks use the approved hierarchy with one tool rail", () => {
  const blocks: Block[] = [
    { kind: "user", text: "ask" },
    { kind: "reasoning", text: "think", expanded: true },
    { kind: "tool", name: "read_file", target: "a.ts", right: "1 line", tone: "ok" },
    { kind: "answer", text: "done" },
  ];
  const drawn = strip(renderAll(blocks, 50, STEEL)).join("\n");
  assert.match(drawn, / ask/);
  assert.match(drawn, / think/);
  assert.match(drawn, /✓ read_file\s+a\.ts/);
  assert.match(drawn, / done/);
  assert.match(drawn, /│|✓ read_file/);
  assert.doesNotMatch(drawn, /[█├└]/);
});

test("tool identity is semantic rather than encoded in decorative glyphs", () => {
  const { blocks, events } = stage([]);
  const names = [
    "read_file",
    "list_dir",
    "find_files",
    "search_text",
    "edit_file",
    "write_file",
    "run_command",
    "invented",
  ];

  for (const name of names) events.onToolCall(callOf(name, name, {}));

  const tools = blocks.filter((block) => block.kind === "tool");
  assert.deepEqual(tools.map((block) => block.name), names);
  assert.ok(tools.every((block) => !("mark" in block)));
});

test("a secret field shows a dot per character and never the characters", () => {
  const secret = "fixture-credential-value";
  const rows = field.panel(
    { title: [{ text: "paste key" }], editor: edit.of(secret), secret: true },
    60,
    STEEL,
  );

  const drawn = rows.join("\n");
  assert.ok(!drawn.includes(secret), "the key is on screen");
  assert.ok(!drawn.includes("sk-"), "part of the key is on screen");
  assert.equal((drawn.match(/●/g) ?? []).length, secret.length);
});

test("the caret sits where the cursor is, counting dots", () => {
  const state = { text: "abcdef", cursor: 3 };
  const at = field.caret({ title: [], editor: state, secret: true }, 60);
  assert.deepEqual(at, { row: 1, col: 5 });
});

test("a key longer than the terminal scrolls, and the caret stays on screen", () => {
  const long = edit.of("x".repeat(300));
  const shown = { title: [], editor: long, secret: true };

  const at = field.caret(shown, 40);
  assert.ok(at.col < 40, `caret at ${at.col} is off the right edge`);
  for (const line of field.panel(shown, 40, STEEL)) assert.ok(textWidth(line) <= 40);
});

test("a pasted newline ends the paste instead of becoming a second row", () => {
  const pasted = field.oneLine({ text: "sk-abc\ndef\n", cursor: 11 });
  assert.equal(pasted.text, "sk-abcdef");
  assert.equal(pasted.cursor, 9);
});

test("an open field takes the dock and keeps the caret on its own row", () => {
  const frame = compose(
    {
      blocks: [],
      editor: edit.EMPTY,
      scroll: 0,
      pal: STEEL,
      footer: base().footer,
      spin: 0,
      modal: {
        kind: "type",
        field: { title: [{ text: "paste key" }], editor: edit.of("abc"), secret: true },
      },
    },
    { rows: 24, cols: 80 },
  );

  const at = frame.cursor;
  assert.ok(at !== undefined, "no caret while a field is open");
  assert.ok(frame.rows[at.row]?.includes("●"), "the caret is not on the input row");
  assert.equal(at.col, 5);
});

test("an open menu shows no caret at all", () => {
  const frame = compose(
    {
      blocks: [],
      editor: edit.EMPTY,
      scroll: 0,
      pal: STEEL,
      footer: base().footer,
      spin: 0,
      modal: {
        kind: "pick",
        picker: approve.promptFor(callOf("1", "write_file", { path: "notes.md" }), "notes.md", STEEL),
      },
    },
    { rows: 24, cols: 80 },
  );

  assert.equal(frame.cursor, undefined);
});

test("help is a compact dock surface with no transcript caret", () => {
  const frame = compose(
    {
      ...base(),
      modal: { kind: "help" },
    },
    { rows: 24, cols: 80 },
  );

  const shown = strip(frame.rows).join("\n");
  assert.match(shown, /help\s+keyboard controls.*esc close/);
  assert.match(shown, /ctrl\+o\s+toggle reasoning or tool details/);
  assert.doesNotMatch(shown, /\/usage/);
  assert.equal(frame.cursor, undefined);
  assert.equal(frame.rows.length, 24);
});
