// The package version is runtime identity, not configuration.

import { readFileSync } from "node:fs";
import * as path from "node:path";

let cached: string | undefined;

export function applicationVersion(): string {
  if (cached !== undefined) return cached;
  const manifest = JSON.parse(
    readFileSync(path.resolve(import.meta.dirname, "..", "package.json"), "utf8"),
  ) as { version?: unknown };
  if (typeof manifest.version !== "string" || manifest.version === "") {
    throw new Error("package version is missing");
  }
  cached = manifest.version;
  return cached;
}
