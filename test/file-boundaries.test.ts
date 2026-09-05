import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import type { ToolContext } from "../src/tools/index.ts";
import { readFile, runListDir, runReadFile } from "../src/tools/file-read.ts";
import { writeFile } from "../src/tools/file-write.ts";

let ctx: ToolContext;

before(async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), "jecode-test-"));
  ctx = { root };
});

after(async () => {
  await fs.rm(ctx.root, { recursive: true, force: true });
});

test("refuses to touch anything outside the root", async () => {
  await assert.rejects(readFile.run({ path: "../escape.txt" }, ctx), /escapes the workspace root/);
  await assert.rejects(
    writeFile.run({ path: "../escape.txt", content: "no" }, ctx),
    /escapes the workspace root/,
  );
});

test("refuses reads and writes through a junction that leaves the root", async (t) => {
  const outside = await fs.mkdtemp(path.join(os.tmpdir(), "jecode-outside-"));
  await fs.writeFile(path.join(outside, "secret.txt"), "secret", "utf8");
  try {
    try {
      await fs.symlink(outside, path.join(ctx.root, "outside-link"), "junction");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "EPERM") {
        t.skip("creating junctions is unavailable for this account");
        return;
      }
      throw error;
    }

    await assert.rejects(
      readFile.run({ path: "outside-link/secret.txt" }, ctx),
      /escapes the workspace root/,
    );
    await assert.rejects(
      writeFile.run({ path: "outside-link/new.txt", content: "no" }, ctx),
      /symbolic link or junction/,
    );
    await assert.rejects(fs.stat(path.join(outside, "new.txt")));
  } finally {
    await fs.rm(path.join(ctx.root, "outside-link"), { force: true }).catch(() => undefined);
    await fs.rm(outside, { recursive: true, force: true });
  }
});

test("allows an internal directory alias for reads but not writes", async (t) => {
  const direct = path.join(ctx.root, "direct-target");
  const alias = path.join(ctx.root, "direct-alias");
  await fs.mkdir(direct);
  await fs.writeFile(path.join(direct, "inside.txt"), "safe", "utf8");
  try {
    try {
      await fs.symlink(direct, alias, process.platform === "win32" ? "junction" : "dir");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "EPERM") {
        t.skip("creating directory aliases is unavailable for this account");
        return;
      }
      throw error;
    }

    assert.equal((await readFile.run({ path: "direct-alias/inside.txt" }, ctx)).output, "safe");
    await assert.rejects(
      writeFile.run({ path: "direct-alias/new.txt", content: "no" }, ctx),
      /symbolic link or junction/,
    );
    await assert.rejects(fs.stat(path.join(direct, "new.txt")));
  } finally {
    await fs.rm(alias, { force: true }).catch(() => undefined);
    await fs.rm(direct, { recursive: true, force: true });
  }
});

test("read_file rejects a final-component swap to an outside file", async (t) => {
  const area = await fs.mkdtemp(path.join(os.tmpdir(), "jecode-read-race-"));
  const workspace = path.join(area, "workspace");
  const outside = path.join(area, "outside.txt");
  const target = path.join(workspace, "target.txt");
  const probe = path.join(workspace, "probe.txt");
  try {
    await fs.mkdir(workspace);
    await fs.writeFile(target, "inside", "utf8");
    await fs.writeFile(outside, "outside secret", "utf8");
    try {
      await fs.symlink(outside, probe, "file");
      await fs.unlink(probe);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "EPERM") {
        t.skip("creating file symlinks is unavailable for this account");
        return;
      }
      throw error;
    }

    await assert.rejects(
      runReadFile({ path: "target.txt" }, { root: workspace }, {
        async beforeOpen() {
          await fs.rename(target, path.join(workspace, "original.txt"));
          await fs.symlink(outside, target, "file");
        },
      }),
      /changed while it was being read/,
    );
  } finally {
    await fs.rm(area, { recursive: true, force: true });
  }
});

test("list_dir rejects a directory swapped to an outside junction", async (t) => {
  const area = await fs.mkdtemp(path.join(os.tmpdir(), "jecode-list-race-"));
  const workspace = path.join(area, "workspace");
  const target = path.join(workspace, "listed");
  const outside = path.join(area, "outside");
  const probe = path.join(workspace, "probe");
  try {
    await fs.mkdir(target, { recursive: true });
    await fs.mkdir(outside);
    await fs.writeFile(path.join(target, "inside.txt"), "inside", "utf8");
    await fs.writeFile(path.join(outside, "secret.txt"), "outside", "utf8");
    try {
      await fs.symlink(outside, probe, process.platform === "win32" ? "junction" : "dir");
      await fs.rm(probe, { force: true });
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "EPERM") {
        t.skip("creating directory links is unavailable for this account");
        return;
      }
      throw error;
    }

    await assert.rejects(
      runListDir({ path: "listed" }, { root: workspace }, {
        async beforeOpen() {
          await fs.rename(target, path.join(workspace, "original"));
          await fs.symlink(
            outside,
            target,
            process.platform === "win32" ? "junction" : "dir",
          );
        },
      }),
      /directory changed while it was being listed/,
    );
  } finally {
    await fs.rm(area, { recursive: true, force: true });
  }
});
