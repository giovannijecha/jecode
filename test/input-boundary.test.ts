import { test } from "node:test";
import assert from "node:assert/strict";
import {
  boundedInputLines,
  MAX_PROMPT_CODE_UNITS,
  PromptLimitError,
} from "../src/input-boundary.ts";

async function* chunks(...values: Array<string | Uint8Array>): AsyncIterable<string | Uint8Array> {
  for (const value of values) yield value;
}

async function collect(source: AsyncIterable<string>): Promise<string[]> {
  const lines: string[] = [];
  for await (const line of source) lines.push(line);
  return lines;
}

test("batch input accepts the exact prompt limit across split chunks", async () => {
  const half = "x".repeat(MAX_PROMPT_CODE_UNITS / 2);
  const lines = await collect(boundedInputLines(chunks(
    half,
    half,
    "\r",
    "\nnext\r",
    "\nlast",
  )));

  assert.deepEqual(lines, [half + half, "next", "last"]);
});

test("batch input rejects limit plus one before concatenating the final chunk", async () => {
  const atLimit = "x".repeat(MAX_PROMPT_CODE_UNITS);

  await assert.rejects(
    collect(boundedInputLines(chunks(atLimit, "x"))),
    PromptLimitError,
  );
});

test("batch input preserves UTF-8 characters split across byte chunks", async () => {
  const encoded = Buffer.from("first\n😀\r\nlast", "utf8");
  const lines = await collect(boundedInputLines(chunks(
    encoded.subarray(0, 8),
    encoded.subarray(8, 10),
    encoded.subarray(10),
  )));

  assert.deepEqual(lines, ["first", "😀", "last"]);
});
