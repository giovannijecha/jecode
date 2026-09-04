import { after, test } from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
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

test("an oversized Git pointer is ignored without an unbounded read", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-workspace-"));
  roots.push(root);
  await writeFile(path.join(root, ".git"), `gitdir: ${"x".repeat(4_096)}`, "utf8");

  assert.equal(await workspaceLabel(root), displayRoot(root));
});

test("a direct Git worktree pointer remains supported", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-workspace-"));
  roots.push(root);
  const gitDirectory = path.join(root, "git-metadata");
  await mkdir(gitDirectory);
  await writeFile(path.join(gitDirectory, "HEAD"), "ref: refs/heads/feature/worktree\n");
  await writeFile(path.join(root, ".git"), "gitdir: ./git-metadata\n", "utf8");

  assert.match(await workspaceLabel(root), / \(feature\/worktree\)$/);
});

test("a linked Git directory is ignored instead of being followed", async (context) => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-workspace-"));
  const outside = await mkdtemp(path.join(tmpdir(), "jecode-git-outside-"));
  roots.push(root, outside);
  await writeFile(path.join(outside, "HEAD"), "ref: refs/heads/outside\n");
  try {
    await symlink(outside, path.join(root, ".git"), process.platform === "win32" ? "junction" : "dir");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "EPERM") {
      context.skip("creating directory links is unavailable for this account");
      return;
    }
    throw error;
  }

  assert.equal(await workspaceLabel(root), displayRoot(root));
});
