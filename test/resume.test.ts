import { test } from "node:test";
import assert from "node:assert/strict";
import { resumePicker } from "../src/tui/resume.ts";
import { STEEL } from "../src/ui/theme.ts";

test("resume timestamps are shown in the terminal's local time", () => {
  const local = new Date(2026, 8, 1, 9, 35);
  const picker = resumePicker([{
    id: "saved-session",
    createdAt: local.toISOString(),
    updatedAt: local.toISOString(),
    turns: 1,
    preview: "saved question",
    active: false,
  }], STEEL);

  assert.equal(picker.options[0]?.hint, "2026-09-01 09:35");
});
