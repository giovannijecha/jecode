// A tiny cross-process lock for persistent user-store mutations.
//
// Every writer rereads inside the lock. That prevents two Jecode processes
// from replacing unrelated settings, API keys, or rotated OAuth credentials
// with snapshots they cached before the other process wrote.

import { randomUUID } from "node:crypto";
import { mkdir, open, readFile, rmdir, stat, unlink } from "node:fs/promises";
import * as path from "node:path";

const WAIT_MS = 50;
const WAIT_LIMIT_MS = 20_000;
const STALE_MS = 60_000;

export async function withStoreLock<T>(
  file: string,
  body: () => Promise<T>,
  signal?: AbortSignal,
): Promise<T> {
  throwIfAborted(signal);
  const directory = `${file}.lock`;
  const token = `${process.pid}:${randomUUID()}`;
  const started = Date.now();

  while (!(await acquire(directory, token))) {
    if (signal?.aborted === true) throw abortReason(signal);
    if (Date.now() - started >= WAIT_LIMIT_MS) {
      throw new Error(`timed out waiting for ${path.basename(file)}`);
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
    if (await ownerIsAlive(directory)) return;

    // Remove the owner first, then the now-empty directory. `rmdir` cannot
    // erase a fresh lock acquired after another waiter wins this recovery,
    // whereas renaming the shared path can steal that new lock in an ABA race.
    await removeLock(directory);
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code !== "ENOENT" && code !== "EACCES" && code !== "EPERM") throw error;
  }
}

async function ownerIsAlive(directory: string): Promise<boolean> {
  let token: string;
  try {
    token = await readFile(path.join(directory, "owner"), "utf8");
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code === "ENOENT") return false;
    if (code === "EACCES" || code === "EPERM") return true;
    throw error;
  }

  const match = /^([1-9]\d*):/.exec(token.trim());
  if (match === null) return false;
  const pid = Number(match[1]);
  if (!Number.isSafeInteger(pid) || pid > 0x7fff_ffff) return false;

  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    // ESRCH is the one portable proof that the owner no longer exists.
    // Permission failures and unknown platform errors must not authorize a
    // second writer to enter the same store.
    return (error as NodeJS.ErrnoException).code !== "ESRCH";
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
