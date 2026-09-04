import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdirSync, renameSync, unlinkSync, writeFileSync } from "node:fs";
import { mkdir, mkdtemp, rename, rm, symlink, unlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import {
  BoundedFileError,
  readBoundedFile,
  readBoundedFileSync,
  readBoundedText,
  withStableFile,
} from "../src/bounded-file.ts";

test("reads an exact bounded regular file", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-bounded-file-"));
  try {
    const file = path.join(root, "value.txt");
    await writeFile(file, "stable value", "utf8");
    assert.equal(await readBoundedText(file, 12), "stable value");
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("rejects a file above its allocation boundary", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-bounded-file-"));
  try {
    const file = path.join(root, "large.bin");
    await writeFile(file, Buffer.alloc(33));
    await assert.rejects(
      readBoundedFile(file, 32),
      (error: unknown) => error instanceof BoundedFileError && error.kind === "too-large",
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("never follows a symbolic final component", async (context) => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-bounded-file-"));
  try {
    const target = path.join(root, "target.txt");
    const alias = path.join(root, "alias.txt");
    await writeFile(target, "private", "utf8");
    try {
      await symlink(target, alias, "file");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "EPERM") {
        context.skip("creating file symlinks is unavailable for this account");
        return;
      }
      throw error;
    }
    await assert.rejects(
      readBoundedFile(alias, 64),
      (error: unknown) => error instanceof BoundedFileError && error.kind === "unsafe",
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("preserves the caller cancellation reason before opening", async () => {
  const control = new AbortController();
  control.abort(new Error("cancel bounded read"));
  await assert.rejects(
    readBoundedFile("not-opened", 64, { signal: control.signal }),
    /cancel bounded read/,
  );
});

test("normalizes an async final-component replacement during open", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-bounded-file-"));
  const file = path.join(root, "value.txt");
  try {
    await writeFile(file, "stable", "utf8");
    await assert.rejects(
      readBoundedFile(file, 64, {
        async beforeOpen() {
          await rename(file, path.join(root, "original.txt"));
          await mkdir(file);
        },
      }),
      (error: unknown) => error instanceof BoundedFileError &&
        (error.kind === "changed" || error.kind === "unsafe"),
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("normalizes a synchronous final-component replacement during open", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-bounded-file-"));
  const file = path.join(root, "value.txt");
  try {
    await writeFile(file, "stable", "utf8");
    assert.throws(
      () => readBoundedFileSync(file, 64, {
        beforeOpen() {
          renameSync(file, path.join(root, "original.txt"));
          mkdirSync(file);
        },
      }),
      (error: unknown) => error instanceof BoundedFileError &&
        (error.kind === "changed" || error.kind === "unsafe"),
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("normalizes an async disappearance between inspection and open", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-bounded-file-"));
  const file = path.join(root, "value.txt");
  try {
    await writeFile(file, "stable", "utf8");
    await assert.rejects(
      readBoundedFile(file, 64, { beforeOpen: async () => unlink(file) }),
      (error: unknown) => error instanceof BoundedFileError && error.kind === "changed",
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("normalizes a synchronous disappearance between inspection and open", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-bounded-file-"));
  const file = path.join(root, "value.txt");
  try {
    await writeFile(file, "stable", "utf8");
    assert.throws(
      () => readBoundedFileSync(file, 64, { beforeOpen: () => unlinkSync(file) }),
      (error: unknown) => error instanceof BoundedFileError && error.kind === "changed",
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test(
  "normalizes an ancestor replacement after an async descriptor read",
  { skip: process.platform === "win32" },
  async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-bounded-file-"));
  const parent = path.join(root, "parent");
  const file = path.join(parent, "value.txt");
  try {
    await mkdir(parent);
    await writeFile(file, "stable", "utf8");
    await assert.rejects(
      withStableFile(file, { label: "fixture" }, async (handle) => {
        await handle.read(Buffer.alloc(6), 0, 6, 0);
        await rename(parent, path.join(root, "parked"));
        await writeFile(parent, "not a directory", "utf8");
      }),
      (error: unknown) => error instanceof BoundedFileError && error.kind === "changed",
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
  },
);

test(
  "normalizes an ancestor replacement after a synchronous descriptor read",
  { skip: process.platform === "win32" },
  async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-bounded-file-"));
  const parent = path.join(root, "parent");
  const file = path.join(parent, "value.txt");
  try {
    await mkdir(parent);
    await writeFile(file, "stable", "utf8");
    assert.throws(
      () => readBoundedFileSync(file, 64, {
        afterRead() {
          renameSync(parent, path.join(root, "parked"));
          writeFileSync(parent, "not a directory", "utf8");
        },
      }),
      (error: unknown) => error instanceof BoundedFileError && error.kind === "changed",
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
  },
);
