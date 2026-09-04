// Stable ownership boundary for private data directories.

import { constants, lstatSync, realpathSync } from "node:fs";
import { lstat, mkdir, open, realpath } from "node:fs/promises";
import * as path from "node:path";
import { fileIdentity, sameFileIdentity } from "./file-identity.ts";
import type { FileIdentity } from "./file-identity.ts";

export type DirectoryAnchor = Readonly<{
  path: string;
  identity: FileIdentity;
  label: string;
}>;

export async function preparePrivateDirectory(
  directory: string,
  label: string,
  mode = 0o700,
): Promise<DirectoryAnchor> {
  const resolved = path.resolve(directory);
  await mkdir(resolved, { recursive: true, mode });
  const anchor = await captureDirectDirectory(resolved, label);
  await secureDirectoryMode(anchor, mode);
  return anchor;
}

export async function createPrivateDirectory(
  directory: string,
  label: string,
  mode = 0o700,
): Promise<DirectoryAnchor> {
  const resolved = path.resolve(directory);
  await mkdir(resolved, { mode });
  const anchor = await captureDirectDirectory(resolved, label);
  await secureDirectoryMode(anchor, mode);
  return anchor;
}

export async function captureDirectDirectory(
  directory: string,
  label: string,
): Promise<DirectoryAnchor> {
  const resolved = path.resolve(directory);
  const [canonical, named] = await Promise.all([
    realpath(resolved),
    lstat(resolved, { bigint: true }),
  ]);
  if (named.isSymbolicLink() || !named.isDirectory()) {
    throw new Error(`${label} is not a direct directory`);
  }
  const direct = await lstat(canonical, { bigint: true });
  if (
    direct.isSymbolicLink() || !direct.isDirectory() ||
    !sameFileIdentity(fileIdentity(named), fileIdentity(direct))
  ) throw new Error(`${label} changed while it was anchored`);
  return Object.freeze({ path: canonical, identity: fileIdentity(direct), label });
}

export function captureDirectDirectorySync(
  directory: string,
  label: string,
): DirectoryAnchor {
  const resolved = path.resolve(directory);
  const canonical = realpathSync(resolved);
  const named = lstatSync(resolved, { bigint: true });
  if (named.isSymbolicLink() || !named.isDirectory()) {
    throw new Error(`${label} is not a direct directory`);
  }
  const direct = lstatSync(canonical, { bigint: true });
  if (
    direct.isSymbolicLink() || !direct.isDirectory() ||
    !sameFileIdentity(fileIdentity(named), fileIdentity(direct))
  ) throw new Error(`${label} changed while it was anchored`);
  return Object.freeze({ path: canonical, identity: fileIdentity(direct), label });
}

export async function assertDirectoryAnchor(anchor: DirectoryAnchor): Promise<void> {
  const current = await lstat(anchor.path, { bigint: true });
  if (
    current.isSymbolicLink() || !current.isDirectory() ||
    !sameFileIdentity(anchor.identity, fileIdentity(current))
  ) {
    throw new Error(`${anchor.label} changed during use`);
  }
}

export function assertDirectoryAnchorSync(anchor: DirectoryAnchor): void {
  const current = lstatSync(anchor.path, { bigint: true });
  if (
    current.isSymbolicLink() || !current.isDirectory() ||
    !sameFileIdentity(anchor.identity, fileIdentity(current))
  ) {
    throw new Error(`${anchor.label} changed during use`);
  }
}

async function secureDirectoryMode(anchor: DirectoryAnchor, mode: number): Promise<void> {
  if (process.platform === "win32") return;
  const handle = await open(
    anchor.path,
    constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW,
  );
  try {
    const opened = await handle.stat({ bigint: true });
    if (
      !opened.isDirectory() ||
      !sameFileIdentity(anchor.identity, fileIdentity(opened))
    ) throw new Error(`${anchor.label} changed before its permissions were secured`);
    await handle.chmod(mode);
    const secured = await handle.stat({ bigint: true });
    if (
      !secured.isDirectory() ||
      !sameFileIdentity(anchor.identity, fileIdentity(secured))
    ) throw new Error(`${anchor.label} changed while its permissions were secured`);
  } finally {
    await handle.close();
  }
  await assertDirectoryAnchor(anchor);
}
