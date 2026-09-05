import { test } from "node:test";
import assert from "node:assert/strict";
import { sessionPermissions } from "../src/permissions.ts";
import { builtinTools } from "../src/tools/index.ts";
import type { ToolCallBlock } from "../src/types.ts";

function call(name: string, input: Record<string, unknown>): ToolCallBlock {
  return { kind: "tool_call", id: `${name}-1`, name, input };
}

test("session permissions expose safe and dangerous defaults", () => {
  const permissions = sessionPermissions(builtinTools());

  assert.deepEqual(
    permissions.listTools().map(({ name, mode }) => ({ name, mode })),
    [
      { name: "read_file", mode: "allow" },
      { name: "list_dir", mode: "allow" },
      { name: "find_files", mode: "allow" },
      { name: "search_text", mode: "allow" },
      { name: "edit_file", mode: "ask" },
      { name: "write_file", mode: "ask" },
      { name: "run_command", mode: "ask" },
    ],
  );
});

test("denied tools are hidden from the next model turn", () => {
  const permissions = sessionPermissions(builtinTools());

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
  const permissions = sessionPermissions(builtinTools());
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

test("dangerous policies remain adjustable and never revive revoked approvals", () => {
  const permissions = sessionPermissions(builtinTools());
  const command = call("run_command", { command: "npm test" });
  permissions.remember(command);
  assert.equal(permissions.approved(command), true);

  assert.equal(permissions.set("run_command", "deny"), true);
  assert.equal(permissions.approved(command), false);
  assert.deepEqual(permissions.listGrants(), []);
  assert.equal(permissions.set("run_command", "allow"), true);
  assert.equal(permissions.approved(command), true);
  assert.equal(permissions.set("run_command", "ask"), true);
  assert.equal(permissions.approved(command), false);
});

test("reset restores defaults and clears remembered approvals", () => {
  const permissions = sessionPermissions(builtinTools());
  permissions.set("read_file", "deny");
  permissions.remember(call("run_command", { command: "npm test" }));

  permissions.reset();

  assert.equal(permissions.listTools().find((tool) => tool.name === "read_file")?.mode, "allow");
  assert.deepEqual(permissions.listGrants(), []);
});
