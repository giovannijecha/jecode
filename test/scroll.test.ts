import { test } from "node:test";
import assert from "node:assert/strict";
import { preserveOffset } from "../src/tui/scroll.ts";

test("following stays pinned to the newest output", () => {
  assert.equal(preserveOffset(0, true, 20, 25), 0);
});

test("scroll lock compensates for rows appended below the viewport", () => {
  assert.equal(preserveOffset(8, false, 20, 25), 13);
});

test("scroll lock clamps when content or the terminal changes", () => {
  assert.equal(preserveOffset(12, false, 20, 6), 6);
});
