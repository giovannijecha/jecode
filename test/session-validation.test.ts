import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdirSync, renameSync } from "node:fs";
import { mkdir, mkdtemp, rename, rm, symlink, unlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { SessionBucket } from "../src/sessions/bucket.ts";
import { queuedValidation } from "../src/sessions/validation.ts";

test("directory validation shares queued requests but never a completed observation", async () => {
  let checks = 0;
  const validate = queuedValidation(async () => { checks++; });
  const first = validate();
  const second = validate();
  assert.equal(checks, 0);
  assert.equal(first, second);
  await Promise.all([first, second]);
  assert.equal(checks, 1);
  const later = validate();
  assert.notEqual(later, first);
  await later;
  assert.equal(checks, 2);
});

test("a boundary reached during filesystem IO waits for a new validation", async () => {
  const started = [Promise.withResolvers<void>(), Promise.withResolvers<void>()];
  const finish = [Promise.withResolvers<void>(), Promise.withResolvers<void>()];
  let checks = 0;
  const validate = queuedValidation(async () => {
    const index = checks++;
    started[index]!.resolve();
    await finish[index]!.promise;
  });
  const first = validate();
  await started[0]!.promise;
  const later = validate();
  const rejected = assert.rejects(later, /directory replaced/);
  assert.notEqual(later, first);
  await started[1]!.promise;
  finish[0]!.resolve();
  await first;
  finish[1]!.reject(new Error("directory replaced"));
  await rejected;
  assert.equal(checks, 2);
});

test("validation failures reach every waiting caller and do not poison future checks", async () => {
  let checks = 0;
  const validate = queuedValidation(async () => {
    if (++checks === 1) throw new Error("unsafe directory");
  });
  await Promise.all([
    assert.rejects(validate(), /unsafe directory/),
    assert.rejects(validate(), /unsafe directory/),
  ]);
  assert.equal(checks, 1);
  await validate();
  assert.equal(checks, 2);
});

for (const boundary of ["root", "bucket", "session"] as const) {
  test(`queued session validation rejects replacement of its ${boundary} before IO starts`, async () => {
    const root = await mkdtemp(path.join(tmpdir(), "jecode-session-validation-"));
    try {
      const workspace = path.join(root, "workspace");
      const sessions = path.join(root, "sessions");
      await mkdir(workspace);
      const bucket = await SessionBucket.open(workspace, sessions);
      const directory = bucket.directory("fixture");
      await mkdir(directory);
      const anchor = await bucket.captureSession("fixture");
      await Promise.all([bucket.assertSession(anchor), bucket.assertSession(anchor)]);
      const replaced = boundary === "root" ? sessions : boundary === "bucket" ? bucket.anchor.path : directory;
      const pending = assert.rejects(bucket.assertSession(anchor), /changed during use/);
      renameSync(replaced, `${replaced}-old`);
      mkdirSync(directory, { recursive: true });
      await Promise.all([
        pending,
        assert.rejects(bucket.assertSession(anchor), /changed during use/),
      ]);
      if (boundary === "session") {
        const replacement = await bucket.captureSession("fixture");
        await Promise.all([
          bucket.assertSession(replacement),
          assert.rejects(bucket.assertSession(anchor), /changed during use/),
        ]);
      }
    } finally { await rm(root, { recursive: true, force: true }); }
  });
}

test("session validation rejects a junction or symlink replacing a verified directory", async (context) => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-session-validation-link-"));
  let linked: string | undefined;
  try {
    const workspace = path.join(root, "workspace");
    await mkdir(workspace);
    const bucket = await SessionBucket.open(workspace, path.join(root, "sessions"));
    const directory = bucket.directory("fixture");
    await mkdir(directory);
    const anchor = await bucket.captureSession("fixture");
    await bucket.assertSession(anchor);
    const moved = `${directory}-old`;
    await rename(directory, moved);
    try {
      await symlink(moved, directory, process.platform === "win32" ? "junction" : "dir");
      linked = directory;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EPERM") throw error;
      context.skip("creating directory links is unavailable for this account");
      return;
    }
    await Promise.all([
      assert.rejects(bucket.assertSession(anchor), /changed during use/),
      assert.rejects(bucket.assertSession(anchor), /changed during use/),
    ]);
  } finally {
    if (linked !== undefined) await unlink(linked);
    await rm(root, { recursive: true, force: true });
  }
});
