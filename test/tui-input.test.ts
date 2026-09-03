import { test } from "node:test";
import assert from "node:assert/strict";
import * as edit from "../src/tui/editor.ts";
import { applyKey } from "../src/tui/input.ts";
import { decoder } from "../src/tui/keys.ts";
import { MAX_PROMPT_CODE_UNITS, PromptLimitError } from "../src/input-boundary.ts";

const ESC = String.fromCharCode(27);
const DEL = String.fromCharCode(127);

function names(keys: { name: string; text: string }[]): string[] {
  return keys.map((key) => (key.text === "" ? key.name : `${key.name}:${key.text}`));
}

test("groups a printable run into one key instead of one per character", () => {
  const keys = decoder().push("ciao");
  assert.deepEqual(names(keys), ["char:ciao"]);
});

test("reads control bytes as ctrl+letter", () => {
  const keys = decoder().push(String.fromCharCode(3) + String.fromCharCode(21));
  assert.deepEqual(
    keys.map((key) => `${key.ctrl ? "ctrl+" : ""}${key.name}`),
    ["ctrl+c", "ctrl+u"],
  );
});

test("both backspace spellings arrive as backspace", () => {
  assert.deepEqual(names(decoder().push(`${DEL}\b`)), ["backspace", "backspace"]);
});

test("Windows Terminal distinguishes Ctrl+Backspace from plain Backspace", () => {
  assert.deepEqual(
    names(decoder({ ctrlBackspaceIsBs: true }).push(`${DEL}\b`)),
    ["backspace", "deletewordleft"],
  );
});

test("common terminal word-editing sequences become semantic keys", () => {
  const keys = decoder().push(
    `${ESC}[1;5D${ESC}[1;5C${ESC}[3;5~${ESC}[127;5u${ESC}d${ESC}${DEL}`,
  );
  assert.deepEqual(names(keys), [
    "wordleft",
    "wordright",
    "deletewordright",
    "deletewordleft",
    "deletewordright",
    "deletewordleft",
  ]);
});

test("holds an escape sequence split across reads instead of guessing", () => {
  const dec = decoder();
  assert.deepEqual(names(dec.push(`${ESC}[`)), []);
  assert.deepEqual(names(dec.push("A")), ["up"]);
});

test("swallows complete terminal sequences it does not bind", () => {
  const dec = decoder();
  assert.deepEqual(names(dec.push(`${ESC}[2~`)), []);
  assert.deepEqual(names(dec.push(`${ESC}OP`)), []);
  assert.deepEqual(names(dec.push("ok")), ["char:ok"]);
});

test("holds an unbound CSI sequence until its final byte arrives", () => {
  const dec = decoder();
  assert.deepEqual(names(dec.push(`${ESC}[?25`)), []);
  assert.deepEqual(names(dec.push("l")), []);
  assert.deepEqual(names(dec.push("ok")), ["char:ok"]);
});

test("a lone escape only resolves on flush", () => {
  const dec = decoder();
  assert.deepEqual(names(dec.push(ESC)), []);
  assert.deepEqual(names(dec.flush()), ["escape"]);
});

test("a bracketed paste arrives whole, newlines included", () => {
  const dec = decoder();
  const keys = dec.push(`${ESC}[200~one\ntwo${ESC}[201~`);
  assert.deepEqual(names(keys), ["paste:one\ntwo"]);
});

test("a paste split across reads does not leak its terminator", () => {
  const dec = decoder();
  dec.push(`${ESC}[200~alpha`);
  const keys = dec.push(`beta${ESC}[201~`);
  assert.deepEqual(names(keys), ["paste:alphabeta"]);
});

test("bracketed and unbracketed paste enforce the shared prompt limit", () => {
  const atLimit = "x".repeat(MAX_PROMPT_CODE_UNITS);
  const bracketed = decoder();
  assert.deepEqual(
    bracketed.push(`${ESC}[200~${atLimit}${ESC}[201~`).map((key) => [key.name, key.text.length]),
    [["paste", MAX_PROMPT_CODE_UNITS]],
  );

  const split = decoder();
  assert.deepEqual(split.push(`${ESC}[200~${atLimit}`), []);
  assert.deepEqual(names(split.push(`x${ESC}[201~`)), ["input_limit"]);
  assert.deepEqual(names(decoder().push(`${atLimit}x`)), ["input_limit"]);
});

test("an unterminated terminal protocol sequence cannot grow the decoder buffer", () => {
  const dec = decoder();
  const sequence = `${ESC}[${"1".repeat(MAX_PROMPT_CODE_UNITS + 1)}`;

  assert.deepEqual(names(dec.push(sequence)), ["input_limit"]);
  assert.deepEqual(names(dec.push("ok")), ["char:ok"]);
});

test("ctrl+c escapes a bracketed paste whose terminator was lost", () => {
  const dec = decoder();
  dec.push(`${ESC}[200~incomplete paste`);
  assert.deepEqual(dec.flush(), []);

  const interrupted = dec.push(String.fromCharCode(3));
  assert.deepEqual(
    interrupted.map((key) => `${key.ctrl ? "ctrl+" : ""}${key.name}`),
    ["ctrl+c"],
  );
  assert.deepEqual(names(dec.push("ok")), ["char:ok"]);
});

test("pasted terminal controls become inert editor text", () => {
  const state = applyKey(edit.EMPTY, {
    name: "paste",
    text: `one${ESC}]52;c;payload${String.fromCharCode(7)}\ntwo`,
    ctrl: false,
  });

  assert.ok(state !== undefined);
  assert.doesNotMatch(state.text, /[\u0000-\u0009\u000b-\u001f\u007f-\u009f]/u);
  assert.ok(state.text.includes(`${String.fromCodePoint(0x241b)}]52`));
  assert.match(state.text, /\ntwo$/);
});

test("editor inserts at the cursor and deletes behind it", () => {
  let state = edit.insert(edit.EMPTY, "hello");
  state = edit.left(edit.left(state));
  state = edit.insert(state, "XY");
  assert.equal(state.text, "helXYlo");
  state = edit.backspace(state);
  assert.equal(state.text, "helXlo");
});

test("the editor accepts the exact prompt limit and refuses one more insertion", () => {
  const state = edit.insert(edit.EMPTY, "x".repeat(MAX_PROMPT_CODE_UNITS));
  assert.equal(state.text.length, MAX_PROMPT_CODE_UNITS);
  assert.throws(() => edit.insert(state, "x"), PromptLimitError);
  assert.equal(state.text.length, MAX_PROMPT_CODE_UNITS);
});

test("ctrl+left and ctrl+right move between word starts", () => {
  const text = "alpha beta gamma";
  assert.equal(edit.wordRight({ text, cursor: 2 }).cursor, 6);
  assert.equal(edit.wordRight({ text, cursor: 6 }).cursor, 11);
  assert.equal(edit.wordLeft({ text, cursor: 13 }).cursor, 11);
  assert.equal(edit.wordLeft({ text, cursor: 11 }).cursor, 6);
});

test("ctrl+w deletes the word behind the cursor, not the whole line", () => {
  const state = edit.killWord(edit.of("git commit -m wip"));
  assert.equal(state.text, "git commit -m ");
});

test("ctrl+delete removes the next word without joining its neighbours", () => {
  const state = edit.killNextWord({ text: "alpha beta gamma", cursor: 5 });
  assert.deepEqual(state, { text: "alpha gamma", cursor: 5 });
});

test("word deletion treats a line break as one editor boundary", () => {
  const text = "alpha\nbeta";
  assert.deepEqual(edit.killWord({ text, cursor: 6 }), { text: "alphabeta", cursor: 5 });
  assert.deepEqual(edit.killNextWord({ text, cursor: 5 }), { text: "alphabeta", cursor: 5 });
});

test("decoded word deletion edits through the shared keymap", () => {
  let state = edit.of("one two three");
  const left = decoder({ ctrlBackspaceIsBs: true }).push("\b")[0];
  assert.ok(left !== undefined);
  state = applyKey(state, left) ?? state;
  assert.equal(state.text, "one two ");

  state = edit.home(state);
  const right = decoder().push(`${ESC}[3;5~`)[0];
  assert.ok(right !== undefined);
  state = applyKey(state, right) ?? state;
  assert.equal(state.text, "two ");
});

test("ctrl+u keeps what is ahead of the cursor", () => {
  const state = edit.killToStart(edit.home(edit.of("abc")));
  assert.equal(state.text, "abc");
});

test("a mouse wheel report arrives as a pointer, not as keystrokes", () => {
  const keys = decoder().push(`${ESC}[<64;10;5M`);
  assert.equal(keys.length, 1);
  assert.equal(keys[0]?.name, "pointer");
  assert.equal(keys[0]?.pointer?.action, "wheel");
  assert.equal(keys[0]?.pointer?.wheel, "up");
  assert.deepEqual([keys[0]?.pointer?.col, keys[0]?.pointer?.row], [9, 4]);
});

test("a mouse report split across reads is held, not half-read", () => {
  const dec = decoder();
  assert.deepEqual(dec.push(`${ESC}[<65;3`), []);
  const keys = dec.push(";7M");
  assert.equal(keys[0]?.pointer?.wheel, "down");
});
