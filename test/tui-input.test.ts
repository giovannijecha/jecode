import { test } from "node:test";
import assert from "node:assert/strict";
import * as edit from "../src/tui/editor.ts";
import { applyKey } from "../src/tui/input.ts";
import { decoder } from "../src/tui/keys.ts";

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

test("holds an escape sequence split across reads instead of guessing", () => {
  const dec = decoder();
  assert.deepEqual(names(dec.push(`${ESC}[`)), []);
  assert.deepEqual(names(dec.push("A")), ["up"]);
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

test("ctrl+w deletes the word behind the cursor, not the whole line", () => {
  const state = edit.killWord(edit.of("git commit -m wip"));
  assert.equal(state.text, "git commit -m ");
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
