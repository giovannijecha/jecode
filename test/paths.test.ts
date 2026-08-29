import { test } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { displayPath, resolveExistingInRoot, resolveInRoot } from "../src/tools/paths.ts";

const root = path.resolve("/tmp/jecode-root");

test("resolves a relative path inside the root", () => {
  assert.equal(resolveInRoot(root, "src/main.ts"), path.join(root, "src", "main.ts"));
});

test("does not mistake an ordinary dot-dot-prefixed name for traversal", () => {
  assert.equal(resolveInRoot(root, "..cache/file"), path.join(root, "..cache", "file"));
});

test("rejects traversal out of the root", () => {
  assert.throws(() => resolveInRoot(root, "../secrets.txt"), /escapes the workspace root/);
  assert.throws(() => resolveInRoot(root, "src/../../secrets.txt"), /escapes the workspace root/);
});

test("rejects an absolute path outside the root", () => {
  const outside = path.resolve("/tmp/somewhere-else/file.txt");
  assert.throws(() => resolveInRoot(root, outside), /escapes the workspace root/);
});

test("accepts an absolute path that lands inside the root", () => {
  const inside = path.join(root, "docs", "notes.md");
  assert.equal(resolveInRoot(root, inside), inside);
});

test("displays paths relative to the root with forward slashes", () => {
  assert.equal(displayPath(root, path.join(root, "src", "main.ts")), "src/main.ts");
  assert.equal(displayPath(root, root), ".");
});

test("accepts a canonical path reached through an aliased root", async (t) => {
  const area = await fs.mkdtemp(path.join(os.tmpdir(), "jecode-path-alias-"));
  const workspace = path.join(area, "workspace");
  const alias = path.join(area, "alias");
  try {
    await fs.mkdir(workspace);
    await fs.writeFile(path.join(workspace, "inside.txt"), "safe", "utf8");
    try {
      await fs.symlink(workspace, alias, process.platform === "win32" ? "junction" : "dir");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "EPERM") {
        t.skip("creating a directory alias is unavailable for this account");
        return;
      }
      throw error;
    }

    const canonicalRoot = await resolveExistingInRoot(alias, ".");
    const canonical = await fs.realpath(path.join(workspace, "inside.txt"));
    assert.equal(await resolveExistingInRoot(canonicalRoot, canonical), canonical);
  } finally {
    await fs.rm(area, { recursive: true, force: true });
  }
});
