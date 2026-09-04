// Process-crash-safe replacement of one file, using a verified temporary
// sibling and rename. POSIX also receives a best-effort parent-directory sync.

import { lstat, open, rename, rm, stat } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import * as path from "node:path";
import { fileIdentity, sameFileIdentity } from "./file-identity.ts";
import type { FileIdentity } from "./file-identity.ts";

export type AtomicWritePhase = "before-open" | "before-write" | "before-rename" | "before-cleanup";

export type AtomicWriteOptions = {
  mode?: number;
  signal?: AbortSignal;
  validate?(phase: AtomicWritePhase): Promise<void>;
};

export async function atomicWrite(
  file: string,
  content: string,
  options: AtomicWriteOptions = {},
): Promise<void> {
  const temporary = path.join(
    path.dirname(file),
    `.${path.basename(file)}.${process.pid}.${randomUUID()}.tmp`,
  );
  let handle: Awaited<ReturnType<typeof open>> | undefined;
  let identity: FileIdentity | undefined;

  try {
    throwIfAborted(options.signal);
    await options.validate?.("before-open");
    throwIfAborted(options.signal);
    const permissions = options.mode ?? (await existingMode(file));
    throwIfAborted(options.signal);
    handle = await open(temporary, "wx", permissions);
    identity = fileIdentity(await handle.stat({ bigint: true }));
    throwIfAborted(options.signal);
    await options.validate?.("before-write");
    throwIfAborted(options.signal);
    await assertNamedFile(temporary, identity);
    await handle.writeFile(content, "utf8");
    throwIfAborted(options.signal);
    if (permissions !== undefined && process.platform !== "win32") {
      await handle.chmod(permissions);
    }
    await handle.sync();
    throwIfAborted(options.signal);
    identity = fileIdentity(await handle.stat({ bigint: true }));
    await options.validate?.("before-rename");
    throwIfAborted(options.signal);
    await assertNamedFile(temporary, identity);
    throwIfAborted(options.signal);
    await rename(temporary, file);
    const completed = handle;
    handle = undefined;
    await completed.close().catch(() => undefined);
    await syncParentDirectory(file);
  } catch (error) {
    if (handle !== undefined) {
      await handle.truncate(0).catch(() => undefined);
      await handle.sync().catch(() => undefined);
      await handle.close().catch(() => undefined);
      handle = undefined;
    }
    await removeTemporary(temporary, identity, options.validate);
    throw error;
  }
}

async function syncParentDirectory(file: string): Promise<void> {
  if (process.platform === "win32") return;
  let directory: Awaited<ReturnType<typeof open>> | undefined;
  try {
    directory = await open(path.dirname(file), "r");
    await directory.sync();
  } catch {
    // The rename is already committed and some filesystems do not support
    // directory fsync. Do not report a successful replacement as failed.
  } finally {
    await directory?.close().catch(() => undefined);
  }
}

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted !== true) return;
  throw signal.reason instanceof Error ? signal.reason : new Error("interrupted");
}

async function assertNamedFile(file: string, expected: FileIdentity): Promise<void> {
  const details = await lstat(file, { bigint: true });
  if (details.isSymbolicLink() || !sameFileIdentity(expected, fileIdentity(details))) {
    throw new Error("atomic write target changed during replacement");
  }
}

async function removeTemporary(
  file: string,
  expected: FileIdentity | undefined,
  validate: AtomicWriteOptions["validate"],
): Promise<void> {
  if (expected === undefined) return;
  try {
    await validate?.("before-cleanup");
    await assertNamedFile(file, expected);
    await rm(file);
  } catch {
    // A changed path is no longer ours to remove.
  }
}

async function existingMode(file: string): Promise<number | undefined> {
  if (process.platform === "win32") return undefined;
  try {
    return (await stat(file)).mode & 0o777;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
    throw error;
  }
}
