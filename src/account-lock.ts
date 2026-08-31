// A tiny cross-process lock for rotating OAuth credentials.
//
// Refresh tokens may rotate after one use. Two Jecode processes refreshing
// the same account concurrently would make one of them persist a dead token,
// so account mutations serialize through an atomic lock directory.

import { mkdir, open, readFile, rename, rmdir, stat, unlink } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import * as path from "node:path";

const WAIT_MS = 50;
const WAIT_LIMIT_MS = 20_000;
const STALE_MS = 60_000;

export async function withAccountLock<T>(
  accountFile: string,
  body: () => Promise<T>,
  signal?: AbortSignal,
): Promise<T> {
  throwIfAborted(signal);
  const directory = `${accountFile}.lock`;
  const token = `${process.pid}:${randomUUID()}`;
  const started = Date.now();

  while (!(await acquire(directory, token))) {
    if (signal?.aborted === true) throw abortReason(signal);
    if (Date.now() - started >= WAIT_LIMIT_MS) {
      throw new Error("timed out waiting for the account store");
    }
    await recoverStale(directory);
    await wait(WAIT_MS, signal);
  }

  try {
    throwIfAborted(signal);
    return await body();
  } finally {
    await release(directory, token);
  }
}

async function acquire(directory: string, token: string): Promise<boolean> {
  let created = false;
  try {
    await mkdir(directory, { mode: 0o700 });
    created = true;
    const owner = await open(path.join(directory, "owner"), "wx", 0o600);
    try {
      await owner.writeFile(token, "utf8");
      await owner.sync();
    } finally {
      await owner.close();
    }
    return true;
  } catch (error) {
    if (!created && (error as NodeJS.ErrnoException).code === "EEXIST") return false;
    if (created) await removeLock(directory);
    throw error;
  }
}

async function recoverStale(directory: string): Promise<void> {
  try {
    const details = await stat(directory);
    if (Date.now() - details.mtimeMs < STALE_MS) return;

    const quarantined = `${directory}.${randomUUID()}.stale`;
    await rename(directory, quarantined);
    await unlink(path.join(quarantined, "owner")).catch(() => undefined);
    await rmdir(quarantined).catch(() => undefined);
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code !== "ENOENT" && code !== "EACCES" && code !== "EPERM") throw error;
  }
}

async function release(directory: string, token: string): Promise<void> {
  try {
    const owner = path.join(directory, "owner");
    if ((await readFile(owner, "utf8")) !== token) return;
    await unlink(owner);
    await rmdir(directory);
  } catch {
    // A recovered stale lock or an already-removed directory is no longer ours.
  }
}

async function removeLock(directory: string): Promise<void> {
  await unlink(path.join(directory, "owner")).catch(() => undefined);
  await rmdir(directory).catch(() => undefined);
}

function wait(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted === true) {
      reject(abortReason(signal));
      return;
    }
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, ms);
    const onAbort = () => {
      clearTimeout(timer);
      reject(signal === undefined ? new Error("cancelled") : abortReason(signal));
    };
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

function abortReason(signal: AbortSignal): Error {
  return signal.reason instanceof Error ? signal.reason : new Error("cancelled");
}

function throwIfAborted(signal?: AbortSignal): void {
  if (signal?.aborted === true) throw abortReason(signal);
}
