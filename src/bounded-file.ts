// Descriptor-based reads for untrusted local files. Identity-sensitive
// metadata stays bigint so NTFS file IDs cannot collide through rounding.

import { constants } from "node:fs";
import {
  closeSync,
  fstatSync,
  lstatSync,
  openSync,
  readSync,
} from "node:fs";
import { lstat, open } from "node:fs/promises";
import type { BigIntStats } from "node:fs";
import type { FileHandle } from "node:fs/promises";
import { fileIdentity, sameFileIdentity } from "./file-identity.ts";
import type { FileIdentity } from "./file-identity.ts";

export type BoundedFileFailure = "changed" | "too-large" | "unsafe";

export class BoundedFileError extends Error {
  readonly kind: BoundedFileFailure;

  constructor(kind: BoundedFileFailure, message: string) {
    super(message);
    this.name = "BoundedFileError";
    this.kind = kind;
  }
}

export type StableFileOptions = Readonly<{
  label?: string;
  signal?: AbortSignal;
  maxBytes?: number;
  expected?: StableFileExpectation;
  validate?(): void | Promise<void>;
  /** Deterministic race barrier used by filesystem tests. */
  beforeOpen?(): void | Promise<void>;
}>;

export type StableFileExpectation = Readonly<{
  identity: FileIdentity;
  size: bigint;
  mtimeNs: bigint;
  ctimeNs: bigint;
}>;

export type BoundedFileOptions = Omit<StableFileOptions, "maxBytes">;

/**
 * Run one reader against a verified regular-file descriptor, then confirm
 * that the descriptor and named path still identify the same stable file.
 */
export async function withStableFile<T>(
  file: string,
  options: StableFileOptions,
  reader: (handle: FileHandle, opened: BigIntStats) => Promise<T>,
): Promise<T> {
  const label = options.label ?? "file";
  requireOptionalLimit(options.maxBytes);
  throwIfAborted(options.signal);

  await options.validate?.();
  const before = await lstat(file, { bigint: true });
  requireRegular(before, label);
  requireSize(before, options.maxBytes, label);
  if (options.expected !== undefined && !sameExpectation(options.expected, before)) {
    throw changed(label);
  }
  await options.beforeOpen?.();
  await options.validate?.();
  throwIfAborted(options.signal);

  let handle: FileHandle;
  try {
    handle = await open(file, readFlags());
  } catch (error) {
    if (unsafeOpenError(error)) throw changed(label);
    throw error;
  }

  try {
    const opened = await handle.stat({ bigint: true });
    requireRegular(opened, label);
    requireSize(opened, options.maxBytes, label);
    if (!sameIdentity(before, opened)) throw changed(label);
    await options.validate?.();

    const value = await reader(handle, opened);
    throwIfAborted(options.signal);
    await options.validate?.();
    const after = await handle.stat({ bigint: true });
    let linked: BigIntStats;
    try {
      linked = await lstat(file, { bigint: true });
    } catch (error) {
      if (unsafeOpenError(error)) throw changed(label);
      throw error;
    }
    if (
      !stableMetadata(opened, after) || !sameIdentity(opened, after) ||
      linked.isSymbolicLink() || !linked.isFile() ||
      !stableMetadata(opened, linked) || !sameIdentity(opened, linked)
    ) throw changed(label);
    await options.validate?.();
    return value;
  } finally {
    await handle.close();
  }
}

/** Open, verify, and read one complete bounded regular file. */
export async function readBoundedFile(
  file: string,
  maxBytes: number,
  options: BoundedFileOptions = {},
): Promise<Buffer> {
  requireLimit(maxBytes);
  return withStableFile(file, { ...options, maxBytes }, async (handle, opened) => {
    const expected = Number(opened.size);
    const capacity = Math.min(maxBytes + 1, Math.max(1, expected + 1));
    const bytes = Buffer.allocUnsafe(capacity);
    let offset = 0;
    while (offset < capacity) {
      throwIfAborted(options.signal);
      const read = await handle.read(bytes, offset, capacity - offset, offset);
      if (read.bytesRead === 0) break;
      offset += read.bytesRead;
    }
    if (offset > maxBytes || offset !== expected) throw changed(options.label ?? "file");
    return bytes.subarray(0, offset);
  });
}

export async function readBoundedText(
  file: string,
  maxBytes: number,
  options: BoundedFileOptions = {},
): Promise<string> {
  return (await readBoundedFile(file, maxBytes, options)).toString("utf8");
}

export type BoundedFileSyncOptions = Readonly<{
  label?: string;
  validate?(): void;
  /** Deterministic race hook for the synchronous startup-store tests. */
  beforeOpen?(): void;
  /** Deterministic post-read race hook for the filesystem tests. */
  afterRead?(): void;
}>;

export function stableFileExpectation(details: BigIntStats): StableFileExpectation {
  return Object.freeze({
    identity: fileIdentity(details),
    size: details.size,
    mtimeNs: details.mtimeNs,
    ctimeNs: details.ctimeNs,
  });
}

/** Synchronous companion for tiny startup stores whose public APIs are sync. */
export function readBoundedFileSync(
  file: string,
  maxBytes: number,
  options: string | BoundedFileSyncOptions = "file",
): Buffer {
  requireLimit(maxBytes);
  const normalized = typeof options === "string" ? { label: options } : options;
  const label = normalized.label ?? "file";
  normalized.validate?.();
  const before = lstatSync(file, { bigint: true });
  requireRegular(before, label);
  requireSize(before, maxBytes, label);
  normalized.beforeOpen?.();
  normalized.validate?.();

  let descriptor: number;
  try {
    descriptor = openSync(file, readFlags());
  } catch (error) {
    if (unsafeOpenError(error)) throw changed(label);
    throw error;
  }
  try {
    const opened = fstatSync(descriptor, { bigint: true });
    requireRegular(opened, label);
    requireSize(opened, maxBytes, label);
    if (!sameIdentity(before, opened)) throw changed(label);
    normalized.validate?.();

    const expected = Number(opened.size);
    const capacity = Math.min(maxBytes + 1, Math.max(1, expected + 1));
    const bytes = Buffer.allocUnsafe(capacity);
    let offset = 0;
    while (offset < capacity) {
      const count = readSync(descriptor, bytes, offset, capacity - offset, offset);
      if (count === 0) break;
      offset += count;
    }

    normalized.afterRead?.();
    const after = fstatSync(descriptor, { bigint: true });
    let linked: BigIntStats;
    try {
      linked = lstatSync(file, { bigint: true });
    } catch (error) {
      if (unsafeOpenError(error)) throw changed(label);
      throw error;
    }
    if (
      offset > maxBytes || offset !== expected ||
      !stableMetadata(opened, after) || !sameIdentity(opened, after) ||
      linked.isSymbolicLink() || !linked.isFile() ||
      !stableMetadata(opened, linked) || !sameIdentity(opened, linked)
    ) throw changed(label);
    normalized.validate?.();
    return bytes.subarray(0, offset);
  } finally {
    closeSync(descriptor);
  }
}

function readFlags(): string | number {
  return process.platform === "win32"
    ? "r"
    : constants.O_RDONLY | (constants.O_NONBLOCK ?? 0) | (constants.O_NOFOLLOW ?? 0);
}

function requireRegular(details: BigIntStats, label: string): void {
  if (details.isSymbolicLink() || !details.isFile()) {
    throw new BoundedFileError("unsafe", `${label} must be a regular file`);
  }
}

function requireSize(details: BigIntStats, maxBytes: number | undefined, label: string): void {
  if (details.size < 0n || (maxBytes !== undefined && details.size > BigInt(maxBytes))) {
    throw new BoundedFileError("too-large", `${label} exceeds ${maxBytes} bytes`);
  }
}

function requireOptionalLimit(maxBytes: number | undefined): void {
  if (maxBytes !== undefined) requireLimit(maxBytes);
}

function requireLimit(maxBytes: number): void {
  if (!Number.isSafeInteger(maxBytes) || maxBytes < 0 || maxBytes >= Number.MAX_SAFE_INTEGER) {
    throw new RangeError("bounded file limit must be a non-negative safe integer");
  }
}

function sameIdentity(left: BigIntStats, right: BigIntStats): boolean {
  return sameFileIdentity(fileIdentity(left), fileIdentity(right));
}

function stableMetadata(left: BigIntStats, right: BigIntStats): boolean {
  return left.size === right.size && left.mtimeNs === right.mtimeNs &&
    left.ctimeNs === right.ctimeNs;
}

function sameExpectation(expected: StableFileExpectation, current: BigIntStats): boolean {
  return sameFileIdentity(expected.identity, fileIdentity(current)) &&
    expected.size === current.size && expected.mtimeNs === current.mtimeNs &&
    expected.ctimeNs === current.ctimeNs;
}

function unsafeOpenError(error: unknown): boolean {
  const code = (error as NodeJS.ErrnoException).code;
  return code === "ENOENT" || code === "ELOOP" || code === "EISDIR" ||
    code === "ENXIO" || code === "ENOTDIR";
}

function changed(label: string): BoundedFileError {
  return new BoundedFileError("changed", `${label} changed while it was being read`);
}

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted !== true) return;
  throw signal.reason instanceof Error ? signal.reason : new Error("aborted");
}
