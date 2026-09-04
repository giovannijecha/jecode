// Compact workspace identity for the persistent footer.

import { lstat } from "node:fs/promises";
import { homedir } from "node:os";
import * as path from "node:path";
import { readBoundedText, stableFileExpectation } from "../bounded-file.ts";
import {
  assertDirectoryAnchor,
  captureDirectDirectory,
} from "../directory-anchor.ts";
import type { DirectoryAnchor } from "../directory-anchor.ts";

const MAX_GIT_POINTER_BYTES = 4_096;

export async function workspaceLabel(root: string): Promise<string> {
  const shown = displayRoot(root);
  const branch = await findBranch(root);
  return branch === undefined ? shown : `${shown} (${branch})`;
}

export function displayRoot(root: string, home = homedir()): string {
  const resolvedHome = path.resolve(home);
  const absolute = path.resolve(root);
  const relative = path.relative(resolvedHome, absolute);
  const inside =
    relative === "" ||
    (relative !== ".." && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative));
  return inside ? (relative === "" ? "~" : `~${path.sep}${relative}`) : absolute;
}

async function findBranch(root: string): Promise<string | undefined> {
  let directory = path.resolve(root);
  for (;;) {
    const branch = await branchAt(directory);
    if (branch !== undefined) return branch;
    const parent = path.dirname(directory);
    if (parent === directory) return undefined;
    directory = parent;
  }
}

async function branchAt(directory: string): Promise<string | undefined> {
  const marker = path.join(directory, ".git");
  try {
    const info = await lstat(marker, { bigint: true });
    if (info.isSymbolicLink()) return undefined;
    if (info.isDirectory()) {
      const gitDirectory = await captureDirectDirectory(marker, "Git directory");
      return readHead(gitDirectory);
    }
    if (!info.isFile()) return undefined;
    const pointer = await readBoundedText(marker, MAX_GIT_POINTER_BYTES, {
      label: "Git directory pointer",
      expected: stableFileExpectation(info),
    });
    const match = /^gitdir:\s*(.+)\s*$/im.exec(pointer);
    if (match === null) return undefined;
    const gitDirectory = await captureDirectDirectory(
      path.resolve(directory, match[1] as string),
      "Git directory",
    );
    return readHead(gitDirectory);
  } catch {
    return undefined;
  }
}

async function readHead(gitDirectory: DirectoryAnchor): Promise<string | undefined> {
  try {
    const head = (await readBoundedText(
      path.join(gitDirectory.path, "HEAD"),
      MAX_GIT_POINTER_BYTES,
      {
        label: "Git HEAD",
        validate: async () => assertDirectoryAnchor(gitDirectory),
      },
    )).trim();
    const prefix = "ref: refs/heads/";
    if (head.startsWith(prefix)) return head.slice(prefix.length);
    return /^[0-9a-f]{7,}$/i.test(head) ? head.slice(0, 7) : undefined;
  } catch {
    return undefined;
  }
}
