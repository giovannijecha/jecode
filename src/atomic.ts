// Crash-safe replacement of one file, using a verified temporary sibling and
// rename.

import { lstat, open, rename, rm, stat } from "node:fs/promises";
import { randomUUID } from "node:crypto";
import * as path from "node:path";

export type AtomicWritePhase = "before-open" | "before-write" | "before-rename" | "before-cleanup";

export type AtomicWriteOptions = {
  mode?: number;
  signal?: AbortSignal;
  validate?(phase: AtomicWritePhase): Promise<void>;
};

type FileIdentity = { dev: number; ino: number; birthtimeMs: number };

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
    identity = fileIdentity(await handle.stat());
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
    identity = fileIdentity(await handle.stat());
    await options.validate?.("before-rename");
    throwIfAborted(options.signal);
    await assertNamedFile(temporary, identity);
    throwIfAborted(options.signal);
    await rename(temporary, file);
    const completed = handle;
    handle = undefined;
    await completed.close().catch(() => undefined);
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

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted !== true) return;
  throw signal.reason instanceof Error ? signal.reason : new Error("interrupted");
}

async function assertNamedFile(file: string, expected: FileIdentity): Promise<void> {
  const details = await lstat(file);
  if (details.isSymbolicLink() || !sameFile(expected, fileIdentity(details))) {
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

function fileIdentity(details: { dev: number; ino: number; birthtimeMs: number }): FileIdentity {
  return { dev: details.dev, ino: details.ino, birthtimeMs: details.birthtimeMs };
}

function sameFile(left: FileIdentity, right: FileIdentity): boolean {
  if (left.dev !== right.dev || left.ino !== right.ino) return false;
  return left.ino !== 0 || left.birthtimeMs === right.birthtimeMs;
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
