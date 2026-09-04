// A tiny cross-process lock for persistent user-store mutations.
//
// Every writer rereads inside the lock. That prevents two Jecode processes
// from replacing unrelated settings, API keys, or rotated OAuth credentials
// with snapshots they cached before the other process wrote.

import * as path from "node:path";
import { lstat } from "node:fs/promises";
import { BoundedFileError, readBoundedText } from "./bounded-file.ts";
import {
  pidIsAlive,
  ProcessLeaseError,
  processLeaseToken,
  tryAcquireProcessLease,
} from "./process-lease.ts";
import type { ProcessLease } from "./process-lease.ts";

const WAIT_MS = 50;
const WAIT_LIMIT_MS = 20_000;
const STALE_MS = 60_000;
const LEGACY_OWNER_BYTES = 256;

export async function withStoreLock<T>(
  file: string,
  body: () => Promise<T>,
  signal?: AbortSignal,
  validate?: () => Promise<void>,
): Promise<T> {
  throwIfAborted(signal);
  await validate?.();
  const directory = `${file}.lock`;
  const token = processLeaseToken();
  const started = Date.now();
  let lease: ProcessLease | undefined;

  while (lease === undefined) {
    await validate?.();
    try {
      lease = await tryAcquireProcessLease(directory, token, {
        staleMs: STALE_MS,
        setupGraceMs: 1_000,
      });
    } catch (error) {
      if (!(error instanceof ProcessLeaseError)) throw error;
      throw await incompatibleLock(directory, file);
    }
    if (lease !== undefined) break;
    if (signal?.aborted === true) throw abortReason(signal);
    if (Date.now() - started >= WAIT_LIMIT_MS) {
      throw new Error(`timed out waiting for ${path.basename(file)}`);
    }
    await wait(WAIT_MS, signal);
  }

  let primaryFailure: unknown;
  try {
    throwIfAborted(signal);
    await lease.assertOwned();
    await validate?.();
    return await body();
  } catch (error) {
    primaryFailure = error;
    throw error;
  } finally {
    try {
      if (!(await lease.release()) && primaryFailure === undefined) {
        throw new Error("store lock ownership was lost");
      }
    } catch (error) {
      if (primaryFailure === undefined) throw error;
    }
  }
}

async function incompatibleLock(directory: string, file: string): Promise<Error> {
  const legacy = await legacyOwner(directory);
  if (legacy !== undefined) {
    if (legacy.pid !== undefined && pidIsAlive(legacy.pid)) {
      return new Error(
        `${path.basename(file)} is locked by an older Jecode process ` +
        `(PID ${legacy.pid}) — close it and retry`,
      );
    }
    return new Error(
      `${path.basename(file)} has an incompatible legacy lock — close older Jecode ` +
      `processes, then remove this exact lock directory and retry: ${directory}`,
    );
  }
  return new Error(
    `${path.basename(file)} has an unsafe lock — inspect this exact directory before retrying: ` +
    directory,
  );
}

async function legacyOwner(directory: string): Promise<{ pid?: number } | undefined> {
  const owner = path.join(directory, "owner");
  try {
    const details = await lstat(owner, { bigint: true });
    if (
      details.isSymbolicLink() || !details.isFile() ||
      details.size > BigInt(LEGACY_OWNER_BYTES)
    ) return { pid: undefined };
    const token = (await readBoundedText(owner, LEGACY_OWNER_BYTES, {
      label: "legacy store lock",
    })).trim();
    const match = /^([1-9]\d*):/.exec(token);
    if (match === null) return { pid: undefined };
    const pid = Number(match[1]);
    return Number.isSafeInteger(pid) && pid <= 0x7fff_ffff
      ? { pid }
      : { pid: undefined };
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
    if (error instanceof BoundedFileError) return { pid: undefined };
    return undefined;
  }
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
