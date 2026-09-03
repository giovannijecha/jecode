// Bounded synchronous reads for the tiny JSON stores under ~/.jecode.

import { Buffer } from "node:buffer";
import { closeSync, constants, fstatSync, openSync, readSync } from "node:fs";

export const USER_STORE_LIMITS = Object.freeze({
  settingsBytes: 64 * 1_024,
  credentialsBytes: 256 * 1_024,
  accountsBytes: 128 * 1_024,
  credentialEntries: 64,
  credentialName: 256,
  credentialValue: 16_384,
  model: 512,
  endpoint: 2_048,
  accountToken: 32_768,
  accountLabel: 1_024,
});

export function readBoundedJsonSync(file: string, maxBytes: number): unknown {
  // Non-blocking open keeps a replaced FIFO or device from stalling startup;
  // fstat below then accepts only regular files before any content is read.
  const flags = process.platform === "win32"
    ? "r"
    : constants.O_RDONLY | (constants.O_NONBLOCK ?? 0);
  const descriptor = openSync(file, flags);
  try {
    const details = fstatSync(descriptor);
    if (!details.isFile()) throw new Error("user store must be a regular file");
    if (!Number.isSafeInteger(details.size) || details.size > maxBytes) {
      throw new Error(`user store exceeds ${maxBytes} bytes`);
    }

    // One extra byte detects an in-place growth after fstat without allowing
    // the read allocation to follow the file's new size.
    const capacity = Math.min(maxBytes + 1, Math.max(1, details.size + 1));
    const bytes = Buffer.allocUnsafe(capacity);
    let offset = 0;
    while (offset < capacity) {
      const count = readSync(descriptor, bytes, offset, capacity - offset, null);
      if (count === 0) break;
      offset += count;
    }
    if (offset > maxBytes || offset > details.size) {
      throw new Error("user store changed while it was being read");
    }
    return JSON.parse(bytes.toString("utf8", 0, offset)) as unknown;
  } finally {
    closeSync(descriptor);
  }
}

export function assertStoreText(text: string, maxBytes: number): void {
  if (Buffer.byteLength(text, "utf8") > maxBytes) {
    throw new Error(`user store exceeds ${maxBytes} bytes`);
  }
}
