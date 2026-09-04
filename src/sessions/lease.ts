// A tiny, generation-safe process lease for one durable conversation.

import { lstat, unlink } from "node:fs/promises";
import {
  BoundedFileError,
  readBoundedText,
  stableFileExpectation,
} from "../bounded-file.ts";
import { fileIdentity, sameFileIdentity } from "../file-identity.ts";
import {
  createProcessLeaseDirectory,
  inspectProcessLease,
  pidIsAlive,
  ProcessLeaseError,
  processLease,
  processLeaseToken,
  removeProcessLease,
  tryAcquireProcessLease,
} from "../process-lease.ts";
import type {
  ProcessLease,
  ProcessLeaseGeneration,
} from "../process-lease.ts";

export type SessionLease = Readonly<{
  id: string;
  assertOwned(): Promise<void>;
  close(): Promise<void>;
}>;

const LEGACY_LEASE_BYTES = 256;

export type SessionLeaseOwner = Readonly<{
  pid: number;
  token: string;
  legacy: boolean;
}>;

export type SessionLeaseInspectHooks = Readonly<{
  /** Deterministic race barrier used by the filesystem test suite. */
  afterLegacyStat?(): void | Promise<void>;
}>;

const sessionLeaseScopes = new WeakMap<object, Readonly<{ id: string; scope: object }>>();

export function leaseToken(): string {
  return processLeaseToken();
}

export async function createLeaseDirectory(
  directory: string,
  token: string,
): Promise<ProcessLeaseGeneration> {
  return createProcessLeaseDirectory(directory, token);
}

export async function claimLeaseDirectory(
  directory: string,
  token: string,
): Promise<ProcessLease | undefined> {
  try {
    return await tryAcquireProcessLease(directory, token, { staleMs: 0, setupGraceMs: 1_000 });
  } catch (error) {
    if (error instanceof ProcessLeaseError) throw unsafeLease();
    throw error;
  }
}

export function leaseFromGeneration(
  directory: string,
  generation: ProcessLeaseGeneration,
): ProcessLease {
  return processLease(directory, generation);
}

export function sessionLease(
  id: string,
  scope: object,
  lease: ProcessLease,
): SessionLease {
  let closed = false;
  const owned = Object.freeze({
    id,
    assertOwned: async () => {
      if (closed) throw new Error("session lease is closed");
      try {
        await lease.assertOwned();
      } catch {
        throw new Error("session lease is no longer owned by this process");
      }
    },
    close: async () => {
      if (closed) return;
      const removed = await lease.release();
      if (!removed) throw new Error("session lease is no longer owned by this process");
      closed = true;
    },
  });
  sessionLeaseScopes.set(owned, { id, scope });
  return owned;
}

export function sessionLeaseOwns(
  lease: SessionLease,
  id: string,
  scope: object,
): boolean {
  const owner = sessionLeaseScopes.get(lease);
  return owner?.id === id && owner.scope === scope;
}

export async function leaseOwner(
  directory: string,
  hooks: SessionLeaseInspectHooks = {},
): Promise<SessionLeaseOwner | undefined> {
  try {
    const owner = await inspectProcessLease(directory);
    return owner === undefined
      ? undefined
      : { pid: owner.pid, token: owner.token, legacy: false };
  } catch (error) {
    if (error instanceof ProcessLeaseError) {
      let legacy: SessionLeaseOwner | undefined;
      try {
        legacy = await legacyLeaseOwner(directory, hooks);
      } catch (legacyError) {
        if (unsafeLegacyRace(legacyError)) throw unsafeLease();
        throw legacyError;
      }
      if (legacy !== undefined) return legacy;
      throw unsafeLease();
    }
    throw error;
  }
}

export async function removeLease(directory: string, token: string): Promise<boolean> {
  try {
    return await removeProcessLease(directory, token);
  } catch (error) {
    if (error instanceof ProcessLeaseError) throw unsafeLease();
    throw error;
  }
}

/** Remove a legacy fixed-file marker only while its session is exclusively owned. */
export async function removeLegacyLeaseExclusive(
  file: string,
  token: string,
  owner: SessionLease,
): Promise<boolean> {
  if (!sessionLeaseScopes.has(owner)) {
    throw new Error("legacy lease migration requires a Jecode session lease");
  }
  await owner.assertOwned();
  let before;
  try {
    before = await lstat(file, { bigint: true });
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return true;
    throw error;
  }
  if (before.isSymbolicLink() || !before.isFile() || before.size > BigInt(LEGACY_LEASE_BYTES)) {
    return false;
  }
  try {
    const current = await readBoundedText(file, LEGACY_LEASE_BYTES, {
      label: "legacy session lease",
      expected: stableFileExpectation(before),
    });
    if (current !== token) return false;
    const after = await lstat(file, { bigint: true });
    if (!sameFileIdentity(fileIdentity(before), fileIdentity(after))) return false;
    await owner.assertOwned();
    await unlink(file);
    return true;
  } catch (error) {
    if (unsafeLegacyRace(error)) return false;
    throw error;
  }
}

export { pidIsAlive };

function unsafeLease(): Error {
  return new Error("session lease is unsafe or too large, or invalid");
}

async function legacyLeaseOwner(
  file: string,
  hooks: SessionLeaseInspectHooks,
): Promise<SessionLeaseOwner | undefined> {
  let details;
  try {
    details = await lstat(file, { bigint: true });
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
    throw error;
  }
  if (details.isSymbolicLink() || !details.isFile() || details.size > BigInt(LEGACY_LEASE_BYTES)) {
    return undefined;
  }
  await hooks.afterLegacyStat?.();
  const token = await readBoundedText(file, LEGACY_LEASE_BYTES, {
    label: "legacy session lease",
    expected: stableFileExpectation(details),
  });
  const match = /^([1-9]\d*):/.exec(token.trim());
  const pid = match === null ? 0 : Number(match[1]);
  return {
    pid: Number.isSafeInteger(pid) && pid <= 0x7fff_ffff ? pid : 0,
    token,
    legacy: true,
  };
}

function unsafeLegacyRace(error: unknown): boolean {
  if (error instanceof BoundedFileError) return true;
  const code = (error as NodeJS.ErrnoException).code;
  return code === "ENOENT" || code === "ENOTDIR" || code === "ELOOP" ||
    code === "EISDIR" || code === "ENXIO";
}
