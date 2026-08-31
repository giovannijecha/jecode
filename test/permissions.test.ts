import { test } from "node:test";
import assert from "node:assert/strict";
import { sessionPermissions } from "../src/permissions.ts";
import { builtinTools } from "../src/tools/index.ts";
import type { ToolCallBlock } from "../src/types.ts";

function call(name: string, input: Record<string, unknown>): ToolCallBlock {
  return { kind: "tool_call", id: `${name}-1`, name, input };
}

test("session permissions expose safe and dangerous defaults", () => {
  const permissions = sessionPermissions(builtinTools(), false);

  assert.deepEqual(
    permissions.listTools().map(({ name, mode, locked }) => ({ name, mode, locked })),
    [
      { name: "read_file", mode: "allow", locked: false },
      { name: "list_dir", mode: "allow", locked: false },
      { name: "find_files", mode: "allow", locked: false },
      { name: "search_text", mode: "allow", locked: false },
      { name: "edit_file", mode: "ask", locked: false },
      { name: "write_file", mode: "ask", locked: false },
      { name: "run_command", mode: "ask", locked: false },
    ],
  );
});

test("denied tools are hidden from the next model turn", () => {
  const permissions = sessionPermissions(builtinTools(), false);

  assert.equal(permissions.set("read_file", "deny"), true);
  assert.equal(permissions.set("run_command", "deny"), true);
  assert.equal(permissions.set("list_dir", "ask"), false);
  assert.equal(permissions.set("unknown", "deny"), false);

  const available = permissions.availableTools().map((tool) => tool.name);
  assert.equal(available.includes("read_file"), false);
  assert.equal(available.includes("run_command"), false);
  assert.equal(available.includes("list_dir"), true);
});

test("dangerous policies and remembered scopes authorize only what they name", () => {
  const permissions = sessionPermissions(builtinTools(), false);
  const first = call("write_file", { path: "a.ts", content: "a" });
  const sameFile = call("edit_file", { path: "a.ts", old_text: "a", new_text: "b" });
  const other = call("write_file", { path: "b.ts", content: "b" });

  assert.equal(permissions.approved(first), false);
  permissions.remember(first);
  assert.equal(permissions.approved(sameFile), true);
  assert.equal(permissions.approved(other), false);
  assert.equal(permissions.listGrants("write_file").length, 1);
  assert.equal(permissions.listGrants("edit_file").length, 1);

  assert.equal(permissions.set("write_file", "allow"), true);
  assert.equal(permissions.approved(other), true);
  assert.deepEqual(permissions.listGrants(), []);
});

test("launch auto-approve locks dangerous tools but not read-only controls", () => {
  const permissions = sessionPermissions(builtinTools(), true);

  const command = permissions.listTools().find((tool) => tool.name === "run_command");
  assert.deepEqual(command, {
    name: "run_command",
    dangerous: true,
    mode: "allow",
    remembered: 0,
    locked: true,
  });
  assert.equal(permissions.set("run_command", "deny"), false);
  assert.equal(permissions.set("read_file", "deny"), true);
});

test("reset restores defaults and clears remembered approvals", () => {
  const permissions = sessionPermissions(builtinTools(), false);
  permissions.set("read_file", "deny");
  permissions.remember(call("run_command", { command: "npm test" }));

  permissions.reset();

  assert.equal(permissions.listTools().find((tool) => tool.name === "read_file")?.mode, "allow");
  assert.deepEqual(permissions.listGrants(), []);
});
