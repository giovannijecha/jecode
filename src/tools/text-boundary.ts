// Shared limits for tools that need to hold an entire text file in memory.

import { constants } from "node:fs";
import { lstat, open } from "node:fs/promises";
import type { FileHandle } from "node:fs/promises";

export const MAX_EDITABLE_BYTES = 4_000_000;
export const MAX_EDITABLE_CHARS = 1_000_000;
export const MAX_EDITABLE_LINES = 20_000;

const READ_CHUNK_BYTES = 64 * 1024;

type ReadOptions = {
  label?: string;
  missingAsEmpty?: boolean;
};

/** Read a regular UTF-8 file without allowing an unbounded allocation. */
export async function readEditableText(
  file: string,
  options: ReadOptions = {},
): Promise<string> {
  const label = options.label ?? "file";
  let details: Awaited<ReturnType<typeof lstat>>;

  try {
    details = await lstat(file);
  } catch (error) {
    if (options.missingAsEmpty === true && isMissing(error)) return "";
    throw error;
  }
  if (!details.isFile()) throw new Error(`${label} must be a regular file`);

  let handle: FileHandle;

  try {
    const flags = process.platform === "win32"
      ? "r"
      : constants.O_RDONLY | (constants.O_NONBLOCK ?? 0);
    handle = await open(file, flags);
  } catch (error) {
    if (options.missingAsEmpty === true && isMissing(error)) return "";
    throw error;
  }

  try {
    const stat = await handle.stat();
    if (!stat.isFile()) throw new Error(`${label} must be a regular file`);
    if (stat.size > MAX_EDITABLE_BYTES) {
      throw limitError(label, `${MAX_EDITABLE_BYTES} UTF-8 bytes`);
    }

    const chunks: Buffer[] = [];
    let total = 0;
    while (total <= MAX_EDITABLE_BYTES) {
      const room = MAX_EDITABLE_BYTES + 1 - total;
      const buffer = Buffer.allocUnsafe(Math.min(READ_CHUNK_BYTES, room));
      const { bytesRead } = await handle.read(buffer, 0, buffer.length, total);
      if (bytesRead === 0) break;
      chunks.push(buffer.subarray(0, bytesRead));
      total += bytesRead;
    }

    if (total > MAX_EDITABLE_BYTES) {
      throw limitError(label, `${MAX_EDITABLE_BYTES} UTF-8 bytes`);
    }

    const text = Buffer.concat(chunks, total).toString("utf8");
    assertEditableText(text, label);
    return text;
  } finally {
    await handle.close();
  }
}

/** Reject whole-file writes that exceed Jecode's mutation budget. */
export function assertEditableText(text: string, label = "content"): void {
  if (Buffer.byteLength(text, "utf8") > MAX_EDITABLE_BYTES) {
    throw limitError(label, `${MAX_EDITABLE_BYTES} UTF-8 bytes`);
  }
  if (text.length > MAX_EDITABLE_CHARS) {
    throw limitError(label, `${MAX_EDITABLE_CHARS} characters`);
  }
  if (lineCount(text) > MAX_EDITABLE_LINES) {
    throw limitError(label, `${MAX_EDITABLE_LINES} lines`);
  }
}

/** Check a replacement's size before constructing the resulting string. */
export function assertReplacementFits(
  before: string,
  oldText: string,
  newText: string,
  replacements: number,
): void {
  const projectedChars =
    before.length + replacements * (newText.length - oldText.length);
  if (projectedChars > MAX_EDITABLE_CHARS) {
    throw limitError("edited content", `${MAX_EDITABLE_CHARS} characters`);
  }

  const projectedLines =
    lineCount(before) +
    replacements * (newlineCount(newText) - newlineCount(oldText));
  if (projectedLines > MAX_EDITABLE_LINES) {
    throw limitError("edited content", `${MAX_EDITABLE_LINES} lines`);
  }
}

function lineCount(text: string): number {
  return newlineCount(text) + 1;
}

function newlineCount(text: string): number {
  let lines = 0;
  for (let index = text.indexOf("\n"); index !== -1; index = text.indexOf("\n", index + 1)) {
    lines += 1;
  }
  return lines;
}

function limitError(label: string, limit: string): Error {
  return new Error(`${label} exceeds the whole-file mutation limit of ${limit}`);
}

function isMissing(error: unknown): boolean {
  return (error as NodeJS.ErrnoException).code === "ENOENT";
}
