import { test } from "node:test";
import assert from "node:assert/strict";
import type { Key } from "../src/tui/keys.ts";
import type { Open } from "../src/tui/overlay.ts";
import { handle } from "../src/tui/overlay.ts";
import * as edit from "../src/tui/editor.ts";

const key = (name: string, text = "", ctrl = false): Key => ({ name, text, ctrl });

test("a searchable overlay settles the original filtered option index", () => {
  let selected: number | undefined;
  let open: Open | undefined = {
    picker: {
      title: [],
      options: [{ label: "alpha" }, { label: "beta" }, { label: "gamma" }],
      searchable: true,
      query: "",
      index: 0,
    },
    settle: (index: number | undefined) => {
      selected = index;
    },
  };
  open = handle(open, key("char", "bet")).open;
  assert.ok(open !== undefined);
  open = handle(open, key("enter")).open;
  assert.equal(open, undefined);
  assert.equal(selected, 1);
});

test("enter cannot pick an invisible row when a filter has no matches", () => {
  let settled = false;
  const open: Open = {
    picker: {
      title: [],
      options: [{ label: "alpha" }],
      searchable: true,
      query: "missing",
      index: 0,
    },
    settle: () => {
      settled = true;
    },
  };
  const outcome = handle(open, key("enter"));
  assert.equal(outcome.open, open);
  assert.equal(settled, false);
});

test("ctrl+c cancels an overlay before asking the activity to abort", () => {
  let answer = 99;
  const open: Open = {
    picker: { title: [], options: [{ label: "one" }], index: 0 },
    settle: (index: number | undefined) => {
      answer = index ?? -1;
    },
  };
  const outcome = handle(open, key("c", "", true));
  assert.equal(answer, -1);
  assert.equal(outcome.open, undefined);
  assert.equal(outcome.abort, true);
});

test("a field remains single-line when pasted text contains newlines", () => {
  let open: Open | undefined = {
    field: { title: [], editor: edit.EMPTY, secret: true },
    settle: () => {},
  };
  open = handle(open, key("paste", "abc\ndef\n")).open;
  assert.ok(open !== undefined && "field" in open);
  assert.equal(open.field.editor.text, "abcdef");
});
