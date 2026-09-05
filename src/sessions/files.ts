// Bounded session-file IO and cleanup of verified temporary directories.

import { randomUUID } from "node:crypto";
import { lstat, opendir, rename, rm } from "node:fs/promises";
import * as path from "node:path";
import { BoundedFileError, readBoundedText, stableFileExpectation } from "../bounded-file.ts";
import type { StableFileExpectation } from "../bounded-file.ts";
import { CONVERSATION_LIMITS } from "../conversation.ts";
import { assertDirectoryAnchor, captureDirectDirectory } from "../directory-anchor.ts";
import type { DirectoryAnchor } from "../directory-anchor.ts";
import { sameFileIdentity } from "../file-identity.ts";
import { decodeNode, SESSION_FILE_LIMITS } from "./codec.ts";
import type { StoredNode } from "./codec.ts";

export const DIRECTORY_MODE = 0o700;
export const FILE_MODE = 0o600;
export const SESSION_NAME = /^[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}$/;

const MAX_NODE_READ_CONCURRENCY = 8;
const MAX_NODE_READ_IN_FLIGHT_BYTES = 64 * 1_024 * 1_024;
const MAX_SESSION_NODE_BYTES = 192 * 1_024 * 1_024;
const NODE_READ_CONCURRENCY = Math.max(1, Math.min(
  MAX_NODE_READ_CONCURRENCY,
  Math.floor(MAX_NODE_READ_IN_FLIGHT_BYTES / SESSION_FILE_LIMITS.nodeBytes),
));
const NODE_NAME = /^(\d{6})\.json$/;
const ATOMIC_NODE_TEMP = /^\.\d{6}\.json\.\d+\.[a-f0-9-]+\.tmp$/;

export async function assertMissingNode(
  file: string,
  validate?: () => Promise<void>,
): Promise<void> {
  await validate?.();
  try {
    await lstat(file);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") {
      await validate?.();
      return;
    }
    throw error;
  }
  await validate?.();
  throw new Error("session has an incomplete node outside its verified snapshot");
}

export async function readNodes(
  directory: DirectoryAnchor,
  validate: () => Promise<void>,
): Promise<StoredNode[]> {
  await validate();
  await assertDirectoryAnchor(directory);
  const names: string[] = [];
  let entries = 0;
  const handle = await opendir(directory.path);
  for await (const entry of handle) {
    entries++;
    if (entries > CONVERSATION_LIMITS.nodes + 64) {
      throw new Error("session node directory contains unsupported data");
    }
    if (
      !entry.isFile() || (!NODE_NAME.test(entry.name) && !ATOMIC_NODE_TEMP.test(entry.name))
    ) {
      throw new Error("session node directory contains unsupported data");
    }
    if (NODE_NAME.test(entry.name)) names.push(entry.name);
  }
  await validate();
  await assertDirectoryAnchor(directory);
  names.sort();
  if (names.length === 0 || names.length > CONVERSATION_LIMITS.nodes) {
    throw new Error("session has an invalid conversation size");
  }
  const files: Array<Readonly<{
    name: string;
    id: number;
    expected: StableFileExpectation;
  }>> = [];
  let storedBytes = 0;
  for (let index = 0; index < names.length; index++) {
    const name = names[index] as string;
    const id = Number(NODE_NAME.exec(name)?.[1]);
    if (id !== index + 1) {
      throw new Error("session conversation nodes are not contiguous");
    }
    const details = await lstat(path.join(directory.path, name), { bigint: true });
    if (
      details.isSymbolicLink() || !details.isFile() || details.size < 0n ||
      details.size > BigInt(SESSION_FILE_LIMITS.nodeBytes)
    ) throw new Error("session node file is unsafe or too large");
    storedBytes += Number(details.size);
    if (storedBytes > MAX_SESSION_NODE_BYTES) {
      throw new Error("session node files exceed their aggregate storage limit");
    }
    files.push({ name, id, expected: stableFileExpectation(details) });
  }
  await validate();
  const stored: StoredNode[] = [];
  const sequences = new Set<number>();
  let messageCodeUnits = 0;
  let transcriptCodeUnits = 0;
  let contextCodeUnits = 0;
  for (let start = 0; start < files.length; start += NODE_READ_CONCURRENCY) {
    const decoded = await Promise.all(files.slice(start, start + NODE_READ_CONCURRENCY)
      .map(async ({ name, id, expected }): Promise<StoredNode> => {
        const entry = decodeNode(await readJson(
          path.join(directory.path, name),
          SESSION_FILE_LIMITS.nodeBytes,
          expected,
        ));
        if (entry.node.id !== id) {
          throw new Error("session conversation node identity is invalid");
        }
        return entry;
      }));
    for (const entry of decoded) {
      if (sequences.has(entry.sequence)) {
        throw new Error("session conversation node identity is invalid");
      }
      messageCodeUnits += JSON.stringify(entry.node.messages).length;
      transcriptCodeUnits += JSON.stringify(entry.node.blocks).length;
      contextCodeUnits += entry.node.context?.summary.length ?? 0;
      if (
        messageCodeUnits > CONVERSATION_LIMITS.messageCodeUnits ||
        transcriptCodeUnits > CONVERSATION_LIMITS.transcriptCodeUnits ||
        contextCodeUnits > CONVERSATION_LIMITS.contextCodeUnits
      ) {
        throw new Error("session conversation exceeds its aggregate limit");
      }
      sequences.add(entry.sequence);
      stored.push(entry);
    }
  }
  await validate();
  return stored;
}

export async function readJson(
  file: string,
  limit: number,
  expected?: StableFileExpectation,
  validate?: () => Promise<void>,
): Promise<unknown> {
  try {
    return JSON.parse(await readBoundedText(file, limit, {
      label: "session file",
      expected,
      validate,
    }));
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new Error("session file is not valid JSON");
    }
    if (error instanceof BoundedFileError) {
      throw new Error("session file is unsafe or too large, or changed while opening");
    }
    throw error;
  }
}

export async function removeTemporaryDirectory(
  directory: string,
  bucket: DirectoryAnchor,
  expected?: DirectoryAnchor,
): Promise<void> {
  const relative = path.relative(bucket.path, directory);
  if (
    relative === "" || relative.startsWith("..") || path.isAbsolute(relative) ||
    !path.basename(directory).startsWith(".") || !path.basename(directory).endsWith(".tmp")
  ) throw new Error("refusing to remove an unverified session directory");
  await assertDirectoryAnchor(bucket);
  let observed: DirectoryAnchor;
  try {
    observed = await captureDirectDirectory(directory, "temporary session directory");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return;
    throw error;
  }
  if (
    expected !== undefined &&
    !sameFileIdentity(expected.identity, observed.identity)
  ) throw new Error("refusing to remove a replaced session directory");

  const quarantine = path.join(
    bucket.path,
    `.discard-${process.pid}-${randomUUID()}.tmp`,
  );
  await assertDirectoryAnchor(bucket);
  await rename(directory, quarantine);
  const moved = await captureDirectDirectory(quarantine, "discarded session directory");
  if (!sameFileIdentity(observed.identity, moved.identity)) {
    throw new Error("session cleanup target changed during quarantine");
  }
  await assertDirectoryAnchor(bucket);
  await assertDirectoryAnchor(moved);
  await rm(quarantine, { recursive: true });
}

export function nodeName(id: number): string {
  return `${String(id).padStart(6, "0")}.json`;
}
