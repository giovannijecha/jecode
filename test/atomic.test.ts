import { test } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import { atomicWrite } from "../src/atomic.ts";
import { assertDirectWritableInRoot } from "../src/tools/paths.ts";

test("revalidates the parent after opening a temporary file and before writing", async (context) => {
  const area = await fs.mkdtemp(path.join(os.tmpdir(), "jecode-atomic-race-"));
  const root = path.join(area, "workspace");
  const parent = path.join(root, "direct");
  const parked = path.join(root, "parked");
  const outside = path.join(area, "outside");
  const target = path.join(parent, "target.txt");
  const probe = path.join(root, "probe");
  const linkType = process.platform === "win32" ? "junction" : "dir";
  await fs.mkdir(parent, { recursive: true });
  await fs.mkdir(outside);

  try {
    try {
      await fs.symlink(outside, probe, linkType);
      await fs.rm(probe, { force: true });
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "EPERM") {
        context.skip("creating directory links is unavailable for this account");
        return;
      }
      throw error;
    }

    let moved = false;
    await assert.rejects(
      atomicWrite(target, "must stay inside", {
        async validate(phase) {
          await assertDirectWritableInRoot(root, target);
          if (phase === "before-open" && !moved) {
            moved = true;
            await fs.rename(parent, parked);
            await fs.symlink(outside, parent, linkType);
          }
        },
      }),
      /symbolic link or junction/,
    );

    await assert.rejects(fs.stat(path.join(outside, "target.txt")));
    const outsideEntries = await fs.readdir(outside);
    assert.ok(outsideEntries.length <= 1);
    for (const name of outsideEntries) {
      assert.equal(await fs.readFile(path.join(outside, name), "utf8"), "");
    }
  } finally {
    await fs.rm(parent, { force: true }).catch(() => undefined);
    await fs.rm(area, { recursive: true, force: true });
  }
});

test("clears temporary content when the parent changes before rename", {
  skip: process.platform === "win32",
}, async () => {
  const area = await fs.mkdtemp(path.join(os.tmpdir(), "jecode-atomic-clear-"));
  const root = path.join(area, "workspace");
  const parent = path.join(root, "direct");
  const parked = path.join(root, "parked");
  const outside = path.join(area, "outside");
  const target = path.join(parent, "target.txt");
  await fs.mkdir(parent, { recursive: true });
  await fs.mkdir(outside);

  try {
    let moved = false;
    await assert.rejects(atomicWrite(target, "must be cleared", {
      async validate(phase) {
        await assertDirectWritableInRoot(root, target);
        if (phase === "before-rename" && !moved) {
          moved = true;
          await fs.rename(parent, parked);
          await fs.symlink(outside, parent, "dir");
        }
      },
    }));

    const parkedEntries = await fs.readdir(parked);
    assert.equal(parkedEntries.length, 1);
    assert.equal(await fs.readFile(path.join(parked, parkedEntries[0] as string), "utf8"), "");
    assert.deepEqual(await fs.readdir(outside), []);
  } finally {
    await fs.rm(parent, { force: true }).catch(() => undefined);
    await fs.rm(area, { recursive: true, force: true });
  }
});
