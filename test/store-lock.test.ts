import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, rm, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import {
  createProcessLeaseDirectory,
  inspectProcessLease,
  processLease,
  processLeaseToken,
  removeObservedProcessLease,
} from "../src/process-lease.ts";
import { withStoreLock } from "../src/store-lock.ts";

test("a stale timestamp never steals a lock from its live owner", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-store-lock-"));
  const file = path.join(root, "accounts.json");
  const directory = `${file}.lock`;
  let entered = false;

  try {
    const generation = await createProcessLeaseDirectory(
      directory,
      `${process.pid}:00000000-0000-4000-8000-000000000001`,
    );
    const old = new Date(Date.now() - 120_000);
    await utimes(path.join(directory, generation.entry), old, old);
    await utimes(directory, old, old);

    await assert.rejects(
      withStoreLock(file, async () => {
        entered = true;
      }, AbortSignal.timeout(150)),
      /aborted|timeout/i,
    );
    assert.equal(entered, false);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("an abandoned stale lock remains recoverable", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-store-lock-"));
  const file = path.join(root, "settings.json");
  const directory = `${file}.lock`;
  let entered = false;

  try {
    const owner = spawn(process.execPath, ["-e", ""], {
      stdio: "ignore",
      windowsHide: true,
    });
    const pid = owner.pid as number;
    await once(owner, "exit");

    const generation = await createProcessLeaseDirectory(
      directory,
      `${pid}:00000000-0000-4000-8000-000000000002`,
    );
    const old = new Date(Date.now() - 120_000);
    await utimes(path.join(directory, generation.entry), old, old);
    await utimes(directory, old, old);

    await withStoreLock(file, async () => {
      entered = true;
    });
    assert.equal(entered, true);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("an unsafe stale owner fails closed instead of being removed", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-store-lock-"));
  const file = path.join(root, "credentials.json");
  const directory = `${file}.lock`;
  let entered = false;

  try {
    const generation = await createProcessLeaseDirectory(
      directory,
      `${process.pid}:00000000-0000-4000-8000-000000000003`,
    );
    await writeFile(path.join(directory, generation.entry), "unsafe", "utf8");
    const old = new Date(Date.now() - 120_000);
    await utimes(directory, old, old);

    await assert.rejects(
      withStoreLock(file, async () => {
        entered = true;
      }),
      /unsafe lock.*credentials\.json\.lock/i,
    );
    assert.equal(entered, false);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("a live legacy store lock fails immediately with upgrade guidance", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-store-lock-"));
  const file = path.join(root, "accounts.json");
  const directory = `${file}.lock`;
  try {
    await mkdir(directory);
    await writeFile(
      path.join(directory, "owner"),
      `${process.pid}:00000000-0000-4000-8000-000000000004`,
      "utf8",
    );

    await assert.rejects(
      withStoreLock(file, async () => undefined),
      /older Jecode process.*close it and retry/i,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("a stale legacy store lock names the exact manual repair boundary", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-store-lock-"));
  const file = path.join(root, "settings.json");
  const directory = `${file}.lock`;
  try {
    await mkdir(directory);
    await writeFile(path.join(directory, "owner"), "99999999:legacy", "utf8");

    await assert.rejects(
      withStoreLock(file, async () => undefined),
      (error: unknown) => error instanceof Error &&
        /incompatible legacy lock/i.test(error.message) &&
        error.message.includes(directory),
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("a lost lock during failure does not hide the store error", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-store-lock-"));
  const file = path.join(root, "settings.json");
  const directory = `${file}.lock`;
  let replacement: ReturnType<typeof processLease> | undefined;
  try {
    await assert.rejects(
      withStoreLock(file, async () => {
        const owner = await inspectProcessLease(directory);
        assert.ok(owner !== undefined);
        assert.equal(await removeObservedProcessLease(directory, owner), true);
        const generation = await createProcessLeaseDirectory(directory, processLeaseToken());
        replacement = processLease(directory, generation);
        throw new Error("primary store failure");
      }),
      /primary store failure/,
    );
    await replacement?.assertOwned();
  } finally {
    await replacement?.release();
    await rm(root, { recursive: true, force: true });
  }
});
