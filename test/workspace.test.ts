import { after, test } from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { displayRoot, workspaceLabel } from "../src/tui/workspace.ts";

const roots: string[] = [];

after(async () => {
  await Promise.all(roots.map((root) => rm(root, { recursive: true, force: true })));
});

test("a workspace below home is compacted without losing its path", () => {
  const home = path.join(path.parse(process.cwd()).root, "Users", "example");
  const root = path.join(home, "Codex", "jecode");
  assert.equal(displayRoot(root, home), `~${path.sep}Codex${path.sep}jecode`);
});

test("the footer discovers the branch from a parent worktree", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-workspace-"));
  roots.push(root);
  await mkdir(path.join(root, ".git"));
  await writeFile(path.join(root, ".git", "HEAD"), "ref: refs/heads/feature/tui\n");
  const nested = path.join(root, "src", "providers");
  await mkdir(nested, { recursive: true });

  assert.match(await workspaceLabel(nested), / \(feature\/tui\)$/);
});
