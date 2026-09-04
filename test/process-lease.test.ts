import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, rm, rmdir, utimes } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import {
  createProcessLeaseDirectory,
  inspectProcessLease,
  processLease,
  processLeaseToken,
  removeObservedProcessLease,
  tryAcquireProcessLease,
} from "../src/process-lease.ts";

test("an observed stale generation cannot remove its replacement", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-process-lease-"));
  const directory = path.join(root, "lease");
  try {
    const firstGeneration = await createProcessLeaseDirectory(directory, processLeaseToken());
    const first = processLease(directory, firstGeneration);
    const observed = await inspectProcessLease(directory);
    assert.ok(observed !== undefined);
    assert.equal(await first.release(), true);

    const secondGeneration = await createProcessLeaseDirectory(directory, processLeaseToken());
    const second = processLease(directory, secondGeneration);
    assert.equal(await removeObservedProcessLease(directory, observed), false);
    await second.assertOwned();
    assert.equal((await inspectProcessLease(directory))?.token, second.token);
    assert.equal(await second.release(), true);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("a recent empty directory is treated as an owner still starting", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-process-lease-"));
  const directory = path.join(root, "lease");
  try {
    await mkdir(directory);
    const lease = await tryAcquireProcessLease(directory, processLeaseToken(), {
      staleMs: 0,
      setupGraceMs: 60_000,
    });
    assert.equal(lease, undefined);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("a delayed creator removes only its entry from a replacement directory", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-process-lease-"));
  const directory = path.join(root, "lease");
  let reachedBarrier!: () => void;
  let leaveBarrier!: () => void;
  const barrier = new Promise<void>((resolve) => {
    reachedBarrier = resolve;
  });
  const resume = new Promise<void>((resolve) => {
    leaveBarrier = resolve;
  });
  try {
    const delayed = createProcessLeaseDirectory(
      directory,
      processLeaseToken(),
      0o700,
      {
        beforeOwnerOpen: async () => {
          reachedBarrier();
          await resume;
        },
      },
    );
    await barrier;
    await rmdir(directory);
    const replacementGeneration = await createProcessLeaseDirectory(
      directory,
      processLeaseToken(),
    );
    const replacement = processLease(directory, replacementGeneration);
    leaveBarrier();

    await assert.rejects(delayed, /changed during acquisition/);
    await replacement.assertOwned();
    assert.equal((await inspectProcessLease(directory))?.token, replacement.token);
    assert.equal(await replacement.release(), true);
  } finally {
    leaveBarrier?.();
    await rm(root, { recursive: true, force: true });
  }
});

test("inspection retries when the owner disappears after enumeration", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-process-lease-"));
  const directory = path.join(root, "lease");
  let reachedBarrier!: () => void;
  let leaveBarrier!: () => void;
  const barrier = new Promise<void>((resolve) => {
    reachedBarrier = resolve;
  });
  const resume = new Promise<void>((resolve) => {
    leaveBarrier = resolve;
  });
  let paused = false;
  try {
    const generation = await createProcessLeaseDirectory(directory, processLeaseToken());
    const lease = processLease(directory, generation);
    const inspected = inspectProcessLease(directory, {
      afterEntries: async () => {
        if (paused) return;
        paused = true;
        reachedBarrier();
        await resume;
      },
    });
    await barrier;
    assert.equal(await lease.release(), true);
    leaveBarrier();

    assert.equal(await inspected, undefined);
  } finally {
    leaveBarrier?.();
    await rm(root, { recursive: true, force: true });
  }
});

test("a dead generation can be recovered without overlapping owners", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "jecode-process-lease-"));
  const directory = path.join(root, "lease");
  try {
    const child = spawn(process.execPath, ["-e", ""], { stdio: "ignore", windowsHide: true });
    const pid = child.pid as number;
    await once(child, "exit");
    const dead = await createProcessLeaseDirectory(
      directory,
      `${pid}:00000000-0000-4000-8000-000000000004`,
    );
    const old = new Date(Date.now() - 120_000);
    await utimes(path.join(directory, dead.entry), old, old);
    const lease = await tryAcquireProcessLease(directory, processLeaseToken(), {
      staleMs: 60_000,
    });
    assert.ok(lease !== undefined);
    await lease.assertOwned();
    assert.equal(await lease.release(), true);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
