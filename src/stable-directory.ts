// Bounded directory enumeration with post-open identity and root checks.

import { lstat, opendir, realpath } from "node:fs/promises";
import type { BigIntStats } from "node:fs";
import * as path from "node:path";
import { stableFileExpectation } from "./bounded-file.ts";
import type { StableFileExpectation } from "./bounded-file.ts";
import { fileIdentity, sameFileIdentity } from "./file-identity.ts";

export class StableDirectoryError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "StableDirectoryError";
  }
}

export type StableDirectoryEntry =
  | Readonly<{ name: string; kind: "file"; expected: StableFileExpectation }>
  | Readonly<{ name: string; kind: "directory" }>;

export type StableDirectoryOptions = Readonly<{
  maxEntries: number;
  signal?: AbortSignal;
  /** Deterministic race barrier used by filesystem tests. */
  beforeOpen?(): void | Promise<void>;
}>;

export async function readStableDirectory(
  root: string,
  directory: string,
  options: StableDirectoryOptions,
): Promise<Readonly<{ entries: StableDirectoryEntry[]; capped: boolean }>> {
  if (!Number.isSafeInteger(options.maxEntries) || options.maxEntries < 0) {
    throw new RangeError("directory entry limit is invalid");
  }
  throwIfAborted(options.signal);
  const canonicalRoot = await realpath(root);
  const canonicalDirectory = await realpath(directory);
  if (!inside(canonicalRoot, canonicalDirectory)) throw escaped();
  const before = await lstat(canonicalDirectory, { bigint: true });
  requireDirectory(before);
  await options.beforeOpen?.();
  throwIfAborted(options.signal);

  const names: string[] = [];
  let capped = false;
  try {
    const handle = await opendir(canonicalDirectory);
    for await (const entry of handle) {
      throwIfAborted(options.signal);
      if (names.length >= options.maxEntries) {
        capped = true;
        break;
      }
      if (directName(entry.name)) names.push(entry.name);
    }
  } catch (error) {
    if (changedPath(error)) throw changed();
    throw error;
  }
  await assertDirectory(canonicalRoot, canonicalDirectory, before);

  const entries: StableDirectoryEntry[] = [];
  for (const name of names) {
    throwIfAborted(options.signal);
    const entry = await inspectEntry(canonicalRoot, canonicalDirectory, name);
    if (entry !== undefined) entries.push(entry);
  }
  await assertDirectory(canonicalRoot, canonicalDirectory, before);
  entries.sort((left, right) => left.name.localeCompare(right.name));
  return { entries, capped };
}

async function inspectEntry(
  root: string,
  directory: string,
  name: string,
): Promise<StableDirectoryEntry | undefined> {
  const target = path.join(directory, name);
  let before: BigIntStats;
  try {
    before = await lstat(target, { bigint: true });
  } catch (error) {
    if (changedPath(error)) return undefined;
    throw error;
  }
  if (before.isSymbolicLink()) return undefined;
  const kind = before.isFile() ? "file" : before.isDirectory() ? "directory" : undefined;
  if (kind === undefined) return undefined;

  let canonical: string;
  try {
    canonical = await realpath(target);
  } catch (error) {
    if (changedPath(error)) return undefined;
    throw error;
  }
  if (!inside(root, canonical) || !samePath(target, canonical)) return undefined;

  let after: BigIntStats;
  try {
    after = await lstat(target, { bigint: true });
  } catch (error) {
    if (changedPath(error)) return undefined;
    throw error;
  }
  if (
    after.isSymbolicLink() ||
    !sameFileIdentity(fileIdentity(before), fileIdentity(after)) ||
    (kind === "file" ? !after.isFile() : !after.isDirectory())
  ) return undefined;
  return kind === "file"
    ? { name, kind, expected: stableFileExpectation(after) }
    : { name, kind };
}

async function assertDirectory(
  root: string,
  directory: string,
  expected: BigIntStats,
): Promise<void> {
  let current: BigIntStats;
  let canonical: string;
  try {
    [current, canonical] = await Promise.all([
      lstat(directory, { bigint: true }),
      realpath(directory),
    ]);
  } catch (error) {
    if (changedPath(error)) throw changed();
    throw error;
  }
  requireDirectory(current);
  if (
    !inside(root, canonical) || !samePath(directory, canonical) ||
    !sameFileIdentity(fileIdentity(expected), fileIdentity(current))
  ) throw changed();
}

function requireDirectory(details: BigIntStats): void {
  if (details.isSymbolicLink() || !details.isDirectory()) throw changed();
}

function directName(name: string): boolean {
  return name !== "." && name !== ".." && path.basename(name) === name;
}

function inside(root: string, target: string): boolean {
  const relative = path.relative(root, target);
  return relative === "" ||
    (!path.isAbsolute(relative) && relative !== ".." && !relative.startsWith(`..${path.sep}`));
}

function samePath(left: string, right: string): boolean {
  return path.relative(left, right) === "";
}

function changedPath(error: unknown): boolean {
  const code = (error as NodeJS.ErrnoException).code;
  return code === "ENOENT" || code === "ENOTDIR" || code === "ELOOP";
}

function changed(): StableDirectoryError {
  return new StableDirectoryError("directory changed while it was being listed");
}

function escaped(): StableDirectoryError {
  return new StableDirectoryError("directory escapes the workspace root");
}

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted !== true) return;
  throw signal.reason instanceof Error ? signal.reason : new Error("aborted");
}
