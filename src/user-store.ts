// Bounded synchronous reads for the tiny JSON stores under ~/.jecode.

import { Buffer } from "node:buffer";
import { readBoundedFileSync } from "./bounded-file.ts";
import type { DirectoryAnchor } from "./directory-anchor.ts";
import { assertDirectoryAnchorSync } from "./directory-anchor.ts";

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

export function readBoundedJsonSync(
  file: string,
  maxBytes: number,
  directory?: DirectoryAnchor,
): unknown {
  const validate = directory === undefined
    ? undefined
    : () => assertDirectoryAnchorSync(directory);
  return JSON.parse(readBoundedFileSync(file, maxBytes, {
    label: "user store",
    validate,
  }).toString("utf8")) as unknown;
}

export function readBoundedJsonForMutationSync(
  file: string,
  maxBytes: number,
  label: string,
  directory?: DirectoryAnchor,
): unknown | undefined {
  try {
    const validate = directory === undefined
      ? undefined
      : () => assertDirectoryAnchorSync(directory);
    return JSON.parse(readBoundedFileSync(file, maxBytes, {
      label: "user store",
      validate,
    }).toString("utf8")) as unknown;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
    throw new Error(`${label} is invalid, unsafe, or too large`, { cause: error });
  }
}

export function assertStoreText(text: string, maxBytes: number): void {
  if (Buffer.byteLength(text, "utf8") > maxBytes) {
    throw new Error(`user store exceeds ${maxBytes} bytes`);
  }
}
