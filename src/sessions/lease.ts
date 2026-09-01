// A tiny, bounded process lease for one durable conversation.

import { randomUUID } from "node:crypto";
import { lstat, open, unlink } from "node:fs/promises";

const MAX_LEASE_BYTES = 256;

export type SessionLease = Readonly<{
  id: string;
  close(): Promise<void>;
}>;

export function leaseToken(): string {
  return `${process.pid}:${randomUUID()}`;
}

export function sessionLease(id: string, file: string, token: string): SessionLease {
  let closed = false;
  return Object.freeze({
    id,
    close: async () => {
      if (closed) return;
      closed = true;
      try {
        await removeLease(file, token);
      } catch {
        // A recovered, replaced, or already-closed lease is no longer ours.
      }
    },
  });
}

export async function leaseOwner(
  file: string,
): Promise<{ pid: number; token: string } | undefined> {
  try {
    const token = await readLease(file);
    const match = /^([1-9]\d*):/.exec(token.trim());
    if (match === null) return { pid: 0, token };
    const pid = Number(match[1]);
    return { pid: Number.isSafeInteger(pid) ? pid : 0, token };
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
    throw error;
  }
}

export async function removeLease(file: string, token: string): Promise<void> {
  const current = await readLease(file).catch(() => undefined);
  if (current === token) await unlink(file).catch(() => undefined);
}

export function pidIsAlive(pid: number): boolean {
  if (!Number.isSafeInteger(pid) || pid < 1 || pid > 0x7fff_ffff) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code !== "ESRCH";
  }
}

async function readLease(file: string): Promise<string> {
  const before = await lstat(file);
  if (before.isSymbolicLink() || !before.isFile() || before.size > MAX_LEASE_BYTES) {
    throw new Error("session lease is unsafe or too large");
  }
  const handle = await open(file, "r");
  try {
    const opened = await handle.stat();
    if (
      !opened.isFile() || opened.size > MAX_LEASE_BYTES ||
      opened.dev !== before.dev || opened.ino !== before.ino
    ) throw new Error("session lease changed while opening");
    const bytes = Buffer.alloc(MAX_LEASE_BYTES + 1);
    const { bytesRead } = await handle.read(bytes, 0, bytes.length, 0);
    if (bytesRead > MAX_LEASE_BYTES) throw new Error("session lease is too large");
    return bytes.subarray(0, bytesRead).toString("utf8");
  } finally {
    await handle.close();
  }
}
