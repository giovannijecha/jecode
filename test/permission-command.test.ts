import { test } from "node:test";
import assert from "node:assert/strict";
import type { Picker } from "../src/tui/picker.ts";
import { handleCommand } from "../src/commands.ts";
import { builtinTools } from "../src/tools/index.ts";
import { sessionPermissions } from "../src/permissions.ts";
import { provider, session, host, texts } from "../dev/test-support/commands.ts";

test("permissions always opens the tool control plane without footer noise", async () => {
  const screen = host(undefined);
  screen.permissions = sessionPermissions(builtinTools(), false);

  await handleCommand("/permissions", session(provider("fake", ["a"])), screen);

  assert.deepEqual(screen.pickers[0]?.options.map((option) => option.label), [
    "read_file",
    "list_dir",
    "find_files",
    "search_text",
    "edit_file",
    "write_file",
    "run_command",
  ]);
  assert.deepEqual(screen.pickers[0]?.title, []);
  assert.equal(screen.pickers[0]?.description, undefined);
  assert.equal(screen.pickers[0]?.visible, 7);
  assert.deepEqual(screen.blocks, []);
});

test("enter on a tool without remembered approvals keeps the control plane open", async () => {
  const screen = host(0, undefined);
  screen.permissions = sessionPermissions(builtinTools(), false);

  await handleCommand("/permissions", session(provider("fake", ["a"])), screen);

  assert.equal(screen.pickers.length, 2);
  assert.deepEqual(screen.pickers[1]?.title, []);
  assert.deepEqual(screen.pickers[1]?.options, screen.pickers[0]?.options);
  assert.deepEqual(screen.blocks, []);
});

test("permissions changes one dangerous tool inline for the session", async () => {
  const screen = host();
  const control = sessionPermissions(builtinTools(), false);
  screen.permissions = control;
  let changed: Picker | undefined;
  screen.choose = (picker) => {
    screen.pickers.push(picker);
    changed = picker.adjust?.(6, 1);
    return Promise.resolve(undefined);
  };

  await handleCommand("/permissions", session(provider("fake", ["a"])), screen);

  assert.equal(control.listTools().find((tool) => tool.name === "run_command")?.mode, "allow");
  assert.equal(changed?.options[6]?.value, "allow");
  assert.equal(changed?.index, 6);
  assert.deepEqual(screen.blocks, []);
});

test("enter reviews and revokes a remembered approval", async () => {
  const screen = host(6, 0, undefined);
  const control = sessionPermissions(builtinTools(), false);
  control.remember({ kind: "tool_call", id: "1", name: "run_command", input: { command: "npm test" } });
  screen.permissions = control;

  await handleCommand("/permissions", session(provider("fake", ["a"])), screen);

  assert.deepEqual(control.listGrants("run_command"), []);
  assert.deepEqual(screen.blocks, []);
});

test("permissions can hide a read-only tool from later turns", async () => {
  const screen = host();
  const control = sessionPermissions(builtinTools(), false);
  screen.permissions = control;
  screen.choose = (picker) => {
    screen.pickers.push(picker);
    picker.adjust?.(0, 1);
    return Promise.resolve(undefined);
  };

  await handleCommand("/permissions", session(provider("fake", ["a"])), screen);

  assert.equal(control.listTools()[0]?.mode, "deny");
  assert.equal(control.availableTools().some((tool) => tool.name === "read_file"), false);
});

test("permissions can revoke every remembered approval for one tool", async () => {
  const screen = host(6, 2, undefined);
  const control = sessionPermissions(builtinTools(), false);
  control.remember({ kind: "tool_call", id: "1", name: "run_command", input: { command: "npm test" } });
  control.remember({ kind: "tool_call", id: "2", name: "run_command", input: { command: "npm run check" } });
  screen.permissions = control;

  await handleCommand("/permissions", session(provider("fake", ["a"])), screen);

  assert.deepEqual(control.listGrants("run_command"), []);
});

test("permissions keeps a launch-time auto-approve override locked inline", async () => {
  const screen = host(6, undefined);
  screen.permissions = sessionPermissions(builtinTools(), true);

  await handleCommand("/permissions", session(provider("fake", ["a"])), screen);

  assert.equal(screen.pickers[0]?.options[6]?.value, "allow · locked");
  assert.equal(screen.pickers[0]?.options[6]?.adjustable, false);
  assert.match(texts(screen.blocks)[0] ?? "", /restart without --auto-approve/);
});
