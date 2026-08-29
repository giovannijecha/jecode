import * as path from "node:path";
import { realpath } from "node:fs/promises";

// Every filesystem tool resolves through here. The agent works inside one
// root and cannot be talked into reaching outside it — including via "..",
// an absolute path, or (on Windows) a different drive letter.
export function resolveInRoot(root: string, candidate: string): string {
  const absoluteRoot = path.resolve(root);
  const absolute = path.resolve(absoluteRoot, candidate);
  if (!inside(absoluteRoot, absolute)) {
    throw new Error(`path escapes the workspace root: ${candidate}`);
  }
  return absolute;
}

/** Resolve an existing path after following symlinks and Windows junctions. */
export async function resolveExistingInRoot(root: string, candidate: string): Promise<string> {
  const lexical = resolveInRoot(root, candidate);
  const canonicalRoot = await realpath(root);
  const canonical = await realpath(lexical);
  if (!inside(canonicalRoot, canonical)) throw new Error(`path escapes the workspace root: ${candidate}`);
  return canonical;
}

/** Resolve a path that may not exist, canonicalizing its nearest existing parent. */
export async function resolveWritableInRoot(root: string, candidate: string): Promise<string> {
  const lexical = resolveInRoot(root, candidate);
  const canonicalRoot = await realpath(root);

  try {
    const canonical = await realpath(lexical);
    if (!inside(canonicalRoot, canonical)) throw new Error(`path escapes the workspace root: ${candidate}`);
    return canonical;
  } catch (error) {
    if (!missing(error)) throw error;
  }

  const rest: string[] = [path.basename(lexical)];
  let parent = path.dirname(lexical);
  while (true) {
    try {
      const canonicalParent = await realpath(parent);
      const target = path.join(canonicalParent, ...rest);
      if (!inside(canonicalRoot, target)) throw new Error(`path escapes the workspace root: ${candidate}`);
      return target;
    } catch (error) {
      if (!missing(error)) throw error;
      const next = path.dirname(parent);
      if (next === parent) throw error;
      rest.unshift(path.basename(parent));
      parent = next;
    }
  }
}

export function displayPath(root: string, absolute: string): string {
  const relative = path.relative(root, absolute);
  return relative === "" ? "." : relative.split(path.sep).join("/");
}

function inside(root: string, target: string): boolean {
  const relative = path.relative(root, target);
  return relative === "" || (!path.isAbsolute(relative) && relative !== ".." && !relative.startsWith(`..${path.sep}`));
}

function missing(error: unknown): boolean {
  return (error as NodeJS.ErrnoException).code === "ENOENT";
}
