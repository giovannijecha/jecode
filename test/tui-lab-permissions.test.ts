import assert from "node:assert/strict";
import { test } from "node:test";
import { createLab } from "../dev/tui/controller.ts";
import type { Lab } from "../dev/tui/controller.ts";
import type { Picker } from "../src/tui/picker.ts";
import { STEEL } from "../src/ui/theme.ts";

function permissionsLab(): Lab {
  return createLab({ scene: "menu-permissions", palette: STEEL, selected: 0, expanded: true, tick: 0 });
}

function picker(lab: Lab): Picker {
  const modal = lab.view().modal;
  assert.ok(modal?.kind === "pick", "permission adjustment must keep the picker open");
  return modal.picker;
}

function select(lab: Lab, name: string): void {
  const index = picker(lab).options.findIndex((option) => option.label === name);
  assert.ok(index >= 0, `missing tool: ${name}`);
  lab.handle({ name: "home", text: "", ctrl: false });
  for (let step = 0; step < index; step++) lab.handle({ name: "down", text: "", ctrl: false });
}

test("the permissions preview cycles read-only policy without closing or moving the picker", () => {
  const lab = permissionsLab();
  try {
    const before = picker(lab).index;
    assert.equal(picker(lab).options[before]?.value, "allow");
    for (const [key, expected] of [["right", "deny"], ["right", "allow"], ["left", "deny"]] as const) {
      lab.handle({ name: key, text: "", ctrl: false });
      assert.equal(picker(lab).index, before);
      assert.equal(picker(lab).options[before]?.value, expected);
    }
  } finally {
    lab.close();
  }
});

test("the permissions preview uses production dangerous modes and clears related remembered grants", () => {
  const lab = permissionsLab();
  try {
    const option = (name: string) => picker(lab).options.find((item) => item.label === name);
    assert.equal(option("edit_file")?.description, "2 remembered");
    assert.equal(option("write_file")?.description, "2 remembered");
    assert.equal(option("run_command")?.description, "1 remembered");

    select(lab, "edit_file");
    for (const [key, expected] of [["right", "allow"], ["right", "deny"], ["right", "ask"], ["left", "deny"]] as const) {
      lab.handle({ name: key, text: "", ctrl: false });
      assert.equal(picker(lab).options[picker(lab).index]?.label, "edit_file");
      assert.equal(option("edit_file")?.value, expected);
    }
    assert.equal(option("edit_file")?.description, undefined);
    assert.equal(option("write_file")?.description, undefined);
    assert.equal(option("run_command")?.description, "1 remembered");
  } finally {
    lab.close();
  }
});

test("permission changes stay local to one preview and reset with the scene", () => {
  const first = permissionsLab();
  const second = permissionsLab();
  try {
    first.handle({ name: "right", text: "", ctrl: false });
    assert.equal(picker(first).options[0]?.value, "deny");
    assert.equal(picker(second).options[0]?.value, "allow");

    first.handle({ name: "escape", text: "", ctrl: false });
    assert.equal(first.view().modal, undefined);
    first.restart();
    assert.equal(picker(first).options[0]?.value, "allow");
    assert.equal(picker(first).options.find((option) => option.label === "edit_file")?.description, "2 remembered");
  } finally {
    first.close();
    second.close();
  }
});
