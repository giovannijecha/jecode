import { test } from "node:test";
import assert from "node:assert/strict";
import * as edit from "../src/tui/editor.ts";
import { compose } from "../src/tui/view.ts";
import { COMMANDS } from "../src/commands.ts";
import { commandMenuLimit } from "../src/tui/components/command-menu.ts";
import { textWidth } from "../src/ui/width.ts";
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
import { base, callOf, strip } from "../dev/test-support/tui.ts";

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
  const settings = COMMANDS.findIndex((command) => command.name === "settings");
  const frame = compose(
    { ...base(), editor: edit.of("/"), menu: COMMANDS, menuIndex: settings },
    { rows: 24, cols: 80 },
  );
  const shown = strip(frame.rows).join("\n");
  assert.match(shown, /● \/settings/);
  assert.doesNotMatch(shown, /^\s*(?:● )?\/help(?:\s|$)/m);
});

test("command suggestions stay between the composer's two rails", () => {
  const frame = compose(
    { ...base(), editor: edit.of("/"), menu: COMMANDS, menuIndex: 0 },
    { rows: 24, cols: 80 },
  );
  const rows = strip(frame.rows);
  const input = frame.cursor?.row ?? -1;
  const first = rows.findIndex((line) => line.includes("/help"));
  const visible = Math.min(commandMenuLimit(), COMMANDS.length);
  const last = rows.findIndex((line) => line.trimStart().startsWith(`/${COMMANDS[visible - 1]!.name}`));
  assert.equal(rows[input - 1], "─".repeat(80));
  assert.match(rows[input] ?? "", new RegExp(`^/.*1–${visible} / ${COMMANDS.length}$`));
  assert.equal(first, input + 1);
  assert.ok(rows[last + 1]?.includes(COMMANDS[0]!.blurb));
  assert.equal(rows[last + 2], "─".repeat(80), "one-line details leave no empty row before the lower rail");
  assert.match(rows[first] ?? "", /^● \/help/);
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
  assert.match(rendered, /^→ beta.*1–2 \/ 2 · 4 total$/m);
  assert.match(rendered, /^● beta/m);
  assert.deepEqual(picker.caret(filtered, 50), { row: 0, col: 6 });
  assert.equal(picker.clear(filtered).query, "");
  const empty = picker.type(source, "missing");
  assert.equal(picker.selected(empty), undefined);
});

test("a searchable picker includes essential row values in its query", () => {
  const source: picker.Picker = {
    title: [],
    options: [
      { label: "shared-model", value: "Anthropic API" },
      { label: "shared-model", value: "OpenAI Account" },
    ],
    searchable: true,
    query: "",
    index: 0,
  };

  const filtered = picker.type(source, "openai account");
  assert.equal(filtered.index, 1);
  assert.equal(picker.selected(filtered), 1);
});

test("a searchable picker removes one complete grapheme", () => {
  const source: picker.Picker = {
    title: [],
    options: [{ label: "family" }],
    searchable: true,
    query: "find 👨‍👩‍👧‍👦",
    index: 0,
  };

  assert.equal(picker.backspace(source).query, "find ");
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
      modal: {
        kind: "pick",
        picker: approve.promptFor(callOf("1", "write_file", { path: "notes.md" }), "notes.md", STEEL),
      },
    },
    { rows: 24, cols: 80 },
  );

  assert.equal(frame.cursor, undefined);
});
