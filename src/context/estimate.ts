// One provider-neutral estimate shared by compaction and request budgeting.

import { constants, deflateRawSync } from "node:zlib";

const BYTES_PER_TOKEN = 3;
const COMPRESSED_BYTES_PER_TOKEN = 5 / 6;

export function estimateSerializedTokens(value: unknown): number {
  const serialized = Buffer.from(JSON.stringify(value), "utf8");
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
