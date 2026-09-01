// Resolve optional native helpers without ever executing workspace content.

import { accessSync, constants, realpathSync, statSync } from "node:fs";
import * as path from "node:path";

export type ExecutableLookup = {
  searchPath?: string;
  cwd?: string;
  rejectUnder?: string;
};

/** Return one canonical executable from PATH, skipping empty and rejected entries. */
export function resolveExecutable(
  name: string,
  options: ExecutableLookup = {},
): string | undefined {
  if (name === "" || path.basename(name) !== name) return undefined;

  const cwd = path.resolve(options.cwd ?? process.cwd());
  const rejected = options.rejectUnder === undefined
    ? undefined
    : canonical(options.rejectUnder);
  const searchPath = options.searchPath ?? process.env["PATH"] ?? "";

  for (const raw of searchPath.split(path.delimiter)) {
    const entry = unquote(raw.trim());
    // An empty PATH entry means the current directory. That is exactly the
    // workspace-controlled lookup this resolver exists to exclude.
    if (entry === "") continue;
    const directory = path.resolve(cwd, entry);
    for (const candidate of executableNames(name)) {
      const executable = usable(path.join(directory, candidate));
      if (executable === undefined) continue;
      if (rejected !== undefined && within(rejected, executable)) continue;
      return executable;
    }
  }
  return undefined;
}

function executableNames(name: string): string[] {
  if (process.platform !== "win32" || path.extname(name) !== "") return [name];
  return [`${name}.com`, `${name}.exe`];
}

function usable(file: string): string | undefined {
  try {
    const resolved = realpathSync.native(file);
    if (!statSync(resolved).isFile()) return undefined;
    accessSync(resolved, process.platform === "win32" ? constants.F_OK : constants.X_OK);
    return resolved;
  } catch {
    return undefined;
  }
}

function canonical(directory: string): string {
  try {
    return realpathSync.native(directory);
  } catch {
    return path.resolve(directory);
  }
}

function within(root: string, candidate: string): boolean {
  const relative = path.relative(root, candidate);
  return relative === "" || (
    relative !== ".." &&
    !relative.startsWith(`..${path.sep}`) &&
    !path.isAbsolute(relative)
  );
}

function unquote(value: string): string {
  if (value.length < 2) return value;
  const first = value[0];
  const last = value[value.length - 1];
  return (first === '"' && last === '"') || (first === "'" && last === "'")
    ? value.slice(1, -1)
    : value;
}
