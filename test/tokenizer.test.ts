import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { countO200k } from "../src/context/tokenizer/o200k.ts";
import { countPiece } from "../src/context/tokenizer/bpe.ts";

test("owned o200k ordinary-text counts match independent tiktoken Unicode and code fixtures", async () => {
  const data = JSON.parse(await readFile(new URL("../dev/test-support/tokenizer-fixtures.json", import.meta.url), "utf8")) as {
    cases: { text: string; tokens: number }[];
  };
  for (const [index, fixture] of data.cases.entries()) {
    assert.equal(await countO200k(fixture.text), fixture.tokens, `reference case ${index}`);
  }
});

test("byte merges choose rank before length and invalidate overlapping candidates", () => {
  // Longest-match would choose ab + cd; BPE first merges the lower-ranked bc.
  const ranks = new Map([["a", 0], ["b", 1], ["c", 2], ["d", 3], ["bc", 4], ["ab", 5], ["cd", 7]]);
  assert.equal(countPiece(Buffer.from("abcd"), ranks), 3);
  assert.equal(countPiece(Buffer.from("aaaa"), new Map([["a", 0], ["aa", 1]])), 2);
});

test("large inputs yield, observe cancellation, and never interpret special token text", async () => {
  const control = new AbortController();
  setImmediate(() => control.abort(new Error("stop tokenizing")));
  await assert.rejects(countO200k("x".repeat(1_000_000), control.signal), /stop tokenizing/);
  const text = "<|endoftext|>";
  assert.equal(await countO200k(text), 7);
  await assert.rejects(countO200k(text, control.signal), /stop tokenizing/);
  assert.ok(await countO200k("a".repeat(16_384)) >= 2_048);
  assert.equal(await countO200k("\uD800"), await countO200k("\uFFFD"));
});
