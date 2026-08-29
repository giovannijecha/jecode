// Canonical per-user storage. Nothing here belongs to a workspace.

import { homedir } from "node:os";
import * as path from "node:path";

export function userDataDir(): string {
  const override = process.env["JECODE_HOME"]?.trim();
  return path.resolve(override === undefined || override === "" ? path.join(homedir(), ".jecode") : override);
}

export function userDataPath(name: string): string {
  return path.join(userDataDir(), name);
}

export function userDataLabel(name: string): string {
  const file = userDataPath(name);
  const relative = path.relative(homedir(), file);
  return relative === ""
    ? "~"
    : relative !== ".." && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative)
      ? `~${path.sep}${relative}`
      : file;
}

/** Previous config-directory location, read only while users move to ~/.jecode. */
export function legacyUserDataPath(name: string): string | undefined {
  const base = process.platform === "win32"
    ? process.env["APPDATA"]?.trim()
    : process.env["XDG_CONFIG_HOME"]?.trim() || path.join(homedir(), ".config");
  if (base === undefined || base === "") return undefined;
  const legacy = path.resolve(base, "jecode", name);
  return legacy === userDataPath(name) ? undefined : legacy;
}
