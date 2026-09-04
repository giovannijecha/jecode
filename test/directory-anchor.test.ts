import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, symlink, unlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import {
  assertDirectoryAnchor,
  captureDirectDirectory,
  captureDirectDirectorySync,
} from "../src/directory-anchor.ts";

test("directory anchors allow an aliased ancestor but reject an aliased leaf", async (context) => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-anchor-"));
  const actual = await mkdtemp(path.join(tmpdir(), "jecode-anchor-target-"));
  const alias = path.join(root, "alias");
  const direct = path.join(actual, "direct");
  const linked = path.join(actual, "linked");
  await mkdir(direct);
  try {
    await symlink(actual, alias, process.platform === "win32" ? "junction" : "dir");
    await symlink(direct, linked, process.platform === "win32" ? "junction" : "dir");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "EPERM") {
      await unlink(linked).catch(() => undefined);
      await unlink(alias).catch(() => undefined);
      await rm(root, { recursive: true, force: true });
      await rm(actual, { recursive: true, force: true });
      context.skip("creating directory links is unavailable for this account");
      return;
    }
    throw error;
  }

  try {
    const asynchronous = await captureDirectDirectory(path.join(alias, "direct"), "fixture");
    const synchronous = captureDirectDirectorySync(path.join(alias, "direct"), "fixture");
    await assertDirectoryAnchor(asynchronous);
    assert.equal(asynchronous.path, synchronous.path);
    await assert.rejects(captureDirectDirectory(linked, "fixture"), /not a direct directory/);
    assert.throws(() => captureDirectDirectorySync(linked, "fixture"), /not a direct directory/);
  } finally {
    await unlink(linked).catch(() => undefined);
    await unlink(alias).catch(() => undefined);
    await rm(root, { recursive: true, force: true });
    await rm(actual, { recursive: true, force: true });
  }
});
