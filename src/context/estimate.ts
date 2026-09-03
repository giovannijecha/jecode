// One provider-neutral estimate shared by compaction and request budgeting.

import { constants, deflateRawSync } from "node:zlib";

const BYTES_PER_TOKEN = 3;
const COMPRESSED_BYTES_PER_TOKEN = 5 / 6;
const SERIALIZED_CHUNK_CODE_UNITS = 64 * 1_024;

export function estimateSerializedTokens(value: unknown): number {
  let tokens = 0;
  for (const serialized of serializedChunks(value)) tokens += estimateChunk(serialized);
  return tokens;
}

/** Estimate the same conservative value while yielding between bounded chunks. */
export async function estimateSerializedTokensResponsive(
  value: unknown,
  signal?: AbortSignal,
): Promise<number> {
  let tokens = 0;
  for (const serialized of serializedChunks(value)) {
    throwIfAborted(signal);
    tokens += estimateChunk(serialized);
    await new Promise<void>((resolve) => setImmediate(resolve));
  }
  throwIfAborted(signal);
  return tokens;
}

function estimateChunk(serialized: Buffer): number {
  const compressedBytes = deflateRawSync(serialized, {
    level: constants.Z_BEST_SPEED,
  }).byteLength;
  const literalTokens = literalTokenFloor(serialized);
  // Normal prose and source code retain the established byte floor. Data that
  // compresses poorly approaches the byte-fallback ceiling used by modern
  // tokenizers. Non-ASCII bytes and punctuation retain a separate literal
  // floor, including when repeated input compresses unusually well.
  return Math.min(serialized.byteLength, Math.max(
    Math.ceil(serialized.byteLength / BYTES_PER_TOKEN),
    Math.ceil(compressedBytes / COMPRESSED_BYTES_PER_TOKEN),
    literalTokens,
  ));
}

function* serializedChunks(value: unknown): Generator<Buffer> {
  const serialized = JSON.stringify(value);
  if (serialized === undefined) throw new TypeError("value is not JSON-serializable");

  for (let start = 0; start < serialized.length;) {
    let end = Math.min(serialized.length, start + SERIALIZED_CHUNK_CODE_UNITS);
    if (
      end < serialized.length &&
      highSurrogate(serialized.charCodeAt(end - 1)) &&
      lowSurrogate(serialized.charCodeAt(end))
    ) end--;
    yield Buffer.from(serialized.slice(start, end), "utf8");
    start = end;
  }
}

function literalTokenFloor(serialized: Buffer): number {
  let compactableAscii = 0;
  let literalBytes = 0;
  for (const byte of serialized) {
    if (
      (byte >= 48 && byte <= 57) ||
      (byte >= 65 && byte <= 90) ||
      (byte >= 97 && byte <= 122)
    ) {
      compactableAscii++;
    } else {
      literalBytes++;
    }
  }
  return literalBytes + Math.ceil(compactableAscii / BYTES_PER_TOKEN);
}

function highSurrogate(value: number): boolean {
  return value >= 0xd800 && value <= 0xdbff;
}

function lowSurrogate(value: number): boolean {
  return value >= 0xdc00 && value <= 0xdfff;
}

function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted !== true) return;
  throw signal.reason instanceof Error ? signal.reason : new Error("aborted");
}
