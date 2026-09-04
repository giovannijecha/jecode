// ABA-safe process ownership built from an atomic directory and one unique
// generation entry. No cleanup operation ever unlinks a shared fixed owner.

import { randomUUID } from "node:crypto";
import { lstat, mkdir, open, opendir, rmdir, unlink } from "node:fs/promises";
import * as path from "node:path";
import { fileIdentity, sameFileIdentity } from "./file-identity.ts";
import type { FileIdentity } from "./file-identity.ts";

const OWNER_NAME = /^owner-([1-9]\d*)-([a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12})$/;
const TOKEN = /^([1-9]\d*):([a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12})$/;

export class ProcessLeaseError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ProcessLeaseError";
  }
}

class ProcessLeaseChangedError extends ProcessLeaseError {}

export type ProcessLeaseGeneration = Readonly<{
  token: string;
  entry: string;
  directoryIdentity: FileIdentity;
  entryIdentity: FileIdentity;
}>;

export type ProcessLeaseOwner = ProcessLeaseGeneration & Readonly<{
  pid: number;
  modifiedAt: number;
}>;

export type ProcessLease = Readonly<{
  token: string;
  assertOwned(): Promise<void>;
  release(): Promise<boolean>;
}>;

export type ProcessLeaseOptions = Readonly<{
  /** A dead generation is recoverable only after this age. */
  staleMs?: number;
  /** Protect the normal mkdir-to-entry publication window. */
  setupGraceMs?: number;
}>;

export type ProcessLeaseCreateHooks = Readonly<{
  /** Deterministic race barrier used by the filesystem test suite. */
  beforeOwnerOpen?(): Promise<void>;
}>;

export type ProcessLeaseInspectHooks = Readonly<{
  /** Deterministic race barrier used by the filesystem test suite. */
  afterEntries?(): Promise<void>;
}>;

export function processLeaseToken(): string {
  return `${process.pid}:${randomUUID()}`;
}

/** Create a new generation in a directory that must not already exist. */
export async function createProcessLeaseDirectory(
  directory: string,
  token: string,
  mode = 0o700,
  hooks: ProcessLeaseCreateHooks = {},
): Promise<ProcessLeaseGeneration> {
  const entry = entryForToken(token);
  await mkdir(directory, { mode });
  const directoryIdentity = await requireDirectoryIdentity(directory);
  let generation: ProcessLeaseGeneration | undefined;
  try {
    await assertDirectoryIdentity(directory, directoryIdentity);
    await hooks.beforeOwnerOpen?.();
    const ownerFile = path.join(directory, entry);
    const handle = await open(ownerFile, "wx", 0o600);
    try {
      const opened = await handle.stat({ bigint: true });
      if (!opened.isFile() || opened.size !== 0n) {
        throw new ProcessLeaseError("process lease owner is unsafe");
      }
      const linked = await lstat(ownerFile, { bigint: true });
      const entryIdentity = fileIdentity(opened);
      if (
        linked.isSymbolicLink() || !linked.isFile() || linked.size !== 0n ||
        !sameFileIdentity(entryIdentity, fileIdentity(linked))
      ) throw new ProcessLeaseError("process lease owner changed while starting");
      generation = Object.freeze({ token, entry, directoryIdentity, entryIdentity });
    } finally {
      await handle.close();
    }
    await assertDirectoryIdentity(directory, directoryIdentity);
    const owner = await inspectProcessLease(directory);
    if (
      owner === undefined || owner.entry !== entry || owner.token !== token ||
      !sameFileIdentity(owner.directoryIdentity, directoryIdentity) ||
      generation === undefined ||
      !sameFileIdentity(owner.entryIdentity, generation.entryIdentity)
    ) throw new ProcessLeaseError("process lease generation lost ownership while starting");
    return generation;
  } catch (error) {
    if (generation !== undefined) {
      await unlinkCreatedEntry(directory, generation).catch(() => undefined);
    }
    await removeEmptyDirectory(directory, directoryIdentity).catch(() => undefined);
    throw error;
  }
}

/** Acquire once, recovering only an observed dead generation. */
export async function tryAcquireProcessLease(
  directory: string,
  token: string,
  options: ProcessLeaseOptions = {},
): Promise<ProcessLease | undefined> {
  const staleMs = boundedDelay(options.staleMs ?? 0);
  const setupGraceMs = boundedDelay(options.setupGraceMs ?? 1_000);

  for (let attempt = 0; attempt < 4; attempt++) {
    try {
      const generation = await createProcessLeaseDirectory(directory, token);
      return processLease(directory, generation);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw error;
    }

    const observation = await inspectDirectory(directory);
    if (observation.kind === "changed") continue;
    if (observation.kind === "missing") continue;
    if (observation.kind === "empty") {
      if (Date.now() - observation.modifiedAt < Math.max(staleMs, setupGraceMs)) {
        return undefined;
      }
      await removeEmptyDirectory(directory, observation.directoryIdentity);
      continue;
    }

    if (
      Date.now() - observation.owner.modifiedAt < staleMs ||
      pidIsAlive(observation.owner.pid)
    ) return undefined;
    await removeObservedProcessLease(directory, observation.owner);
  }
  return undefined;
}

export function processLease(
  directory: string,
  generation: ProcessLeaseGeneration,
): ProcessLease {
  let released = false;
  return Object.freeze({
    token: generation.token,
    assertOwned: async () => {
      if (released) throw new ProcessLeaseError("process lease is closed");
      const owner = await inspectProcessLease(directory);
      if (
        owner === undefined || owner.entry !== generation.entry ||
        owner.token !== generation.token ||
        !sameFileIdentity(owner.directoryIdentity, generation.directoryIdentity) ||
        !sameFileIdentity(owner.entryIdentity, generation.entryIdentity)
      ) throw new ProcessLeaseError("process lease is no longer owned by this process");
    },
    release: async () => {
      if (released) return true;
      const removed = await removeObservedProcessLease(directory, generation);
      if (removed) released = true;
      return removed;
    },
  });
}

export async function inspectProcessLease(
  directory: string,
  hooks: ProcessLeaseInspectHooks = {},
): Promise<ProcessLeaseOwner | undefined> {
  for (let attempt = 0; attempt < 4; attempt++) {
    const observation = await inspectDirectory(directory, hooks);
    if (observation.kind === "changed") continue;
    if (observation.kind === "missing" || observation.kind === "empty") return undefined;
    return observation.owner;
  }
  throw new ProcessLeaseError("process lease kept changing during inspection");
}

/** Remove only this unique generation, never a later owner's entry. */
export async function removeProcessLease(
  directory: string,
  token: string,
): Promise<boolean> {
  const entry = entryForToken(token);
  let owner: ProcessLeaseOwner | undefined;
  try {
    owner = await inspectProcessLease(directory);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return true;
    throw error;
  }
  if (owner === undefined) return false;
  if (owner.entry !== entry || owner.token !== token) return false;
  return removeObservedProcessLease(directory, owner);
}

export async function removeObservedProcessLease(
  directory: string,
  generation: ProcessLeaseGeneration,
): Promise<boolean> {
  try {
    await assertDirectoryIdentity(directory, generation.directoryIdentity);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return true;
    if (error instanceof ProcessLeaseError) return false;
    throw error;
  }
  const removed = await unlinkObserved(directory, generation);
  if (!removed) return false;
  try {
    await rmdir(directory);
    return true;
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code === "ENOENT") return true;
    // Our unique generation is already gone. A delayed or newer generation
    // now owns any remaining entry and must clean up its own directory.
    if (code === "ENOTEMPTY" || code === "EEXIST") return true;
    throw error;
  }
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

type DirectoryObservation =
  | Readonly<{ kind: "missing" }>
  | Readonly<{ kind: "changed" }>
  | Readonly<{ kind: "empty"; directoryIdentity: FileIdentity; modifiedAt: number }>
  | Readonly<{ kind: "owned"; owner: ProcessLeaseOwner }>;

async function inspectDirectory(
  directory: string,
  hooks: ProcessLeaseInspectHooks = {},
): Promise<DirectoryObservation> {
  let details;
  try {
    details = await lstat(directory, { bigint: true });
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return { kind: "missing" };
    throw error;
  }
  if (details.isSymbolicLink() || !details.isDirectory()) {
    throw new ProcessLeaseError("process lease path is not a direct directory");
  }
  const directoryIdentity = fileIdentity(details);
  try {
    const names: string[] = [];
    const handle = await opendir(directory);
    for await (const entry of handle) {
      names.push(entry.name);
      if (names.length > 1) break;
    }
    await assertDirectoryIdentity(directory, directoryIdentity);
    if (names.length === 0) {
      return { kind: "empty", directoryIdentity, modifiedAt: Number(details.mtimeMs) };
    }
    if (names.length !== 1) throw new ProcessLeaseError("process lease has multiple owners");
    await hooks.afterEntries?.();
    const name = names[0] as string;
    const parsed = OWNER_NAME.exec(name);
    if (parsed === null) throw new ProcessLeaseError("process lease owner is invalid");
    const pid = Number(parsed[1]);
    if (!Number.isSafeInteger(pid) || pid > 0x7fff_ffff) {
      throw new ProcessLeaseError("process lease owner is invalid");
    }
    const ownerFile = path.join(directory, name);
    const owner = await lstat(ownerFile, { bigint: true });
    if (owner.isSymbolicLink() || !owner.isFile() || owner.size !== 0n) {
      throw new ProcessLeaseError("process lease owner is unsafe");
    }
    await assertDirectoryIdentity(directory, directoryIdentity);
    return {
      kind: "owned",
      owner: Object.freeze({
        pid,
        token: `${parsed[1]}:${parsed[2]}`,
        entry: name,
        directoryIdentity,
        entryIdentity: fileIdentity(owner),
        modifiedAt: Number(owner.mtimeMs),
      }),
    };
  } catch (error) {
    if (inspectionChanged(error)) return { kind: "changed" };
    throw error;
  }
}

async function unlinkObserved(
  directory: string,
  generation: ProcessLeaseGeneration,
): Promise<boolean> {
  try {
    await assertDirectoryIdentity(directory, generation.directoryIdentity);
    const ownerFile = path.join(directory, generation.entry);
    const owner = await lstat(ownerFile, { bigint: true });
    if (
      owner.isSymbolicLink() || !owner.isFile() || owner.size !== 0n ||
      !sameFileIdentity(fileIdentity(owner), generation.entryIdentity)
    ) return false;
    await unlink(ownerFile);
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return false;
    if (error instanceof ProcessLeaseError) return false;
    throw error;
  }
}

/** Clean up only the unique entry this failed creator actually opened. */
async function unlinkCreatedEntry(
  directory: string,
  generation: ProcessLeaseGeneration,
): Promise<boolean> {
  try {
    const ownerFile = path.join(directory, generation.entry);
    const owner = await lstat(ownerFile, { bigint: true });
    if (
      owner.isSymbolicLink() || !owner.isFile() || owner.size !== 0n ||
      !sameFileIdentity(fileIdentity(owner), generation.entryIdentity)
    ) return false;
    await unlink(ownerFile);
    return true;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return true;
    throw error;
  }
}

async function removeEmptyDirectory(
  directory: string,
  identity: FileIdentity,
): Promise<boolean> {
  try {
    await assertDirectoryIdentity(directory, identity);
    await rmdir(directory);
    return true;
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code === "ENOENT") return true;
    if (code === "ENOTEMPTY" || code === "EEXIST" || error instanceof ProcessLeaseError) {
      return false;
    }
    throw error;
  }
}

async function requireDirectoryIdentity(directory: string): Promise<FileIdentity> {
  const details = await lstat(directory, { bigint: true });
  if (details.isSymbolicLink() || !details.isDirectory()) {
    throw new ProcessLeaseError("process lease path is not a direct directory");
  }
  return fileIdentity(details);
}

async function assertDirectoryIdentity(directory: string, expected: FileIdentity): Promise<void> {
  const current = await requireDirectoryIdentity(directory);
  if (!sameFileIdentity(current, expected)) {
    throw new ProcessLeaseChangedError("process lease directory changed during acquisition");
  }
}

function inspectionChanged(error: unknown): boolean {
  const code = (error as NodeJS.ErrnoException).code;
  return code === "ENOENT" || code === "ENOTDIR" || error instanceof ProcessLeaseChangedError;
}

function entryForToken(token: string): string {
  const parsed = TOKEN.exec(token);
  if (parsed === null) throw new ProcessLeaseError("process lease token is invalid");
  return `owner-${parsed[1]}-${parsed[2]}`;
}

function boundedDelay(value: number): number {
  if (!Number.isSafeInteger(value) || value < 0 || value > 24 * 60 * 60 * 1_000) {
    throw new RangeError("process lease delay is invalid");
  }
  return value;
}
