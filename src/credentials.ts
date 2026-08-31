// Where an API key comes from, and where one goes when the user types it in.
//
// Three layers, looked at in this order: the environment, then whatever this
// session was handed, then the saved file. The environment winning is the
// load-bearing part — a key exported in the shell is the one the user is
// looking at, and a stale saved key quietly overriding it is the worst kind of
// bug to be on the wrong side of.
//
// The file lives under ~/.jecode and never in this repo. A
// secret in the working tree is one `git add -A` from being published, which
// is why "not in the repo" is a rule and not a preference.

import { chmod, mkdir } from "node:fs/promises";
import { readFileSync } from "node:fs";
import * as path from "node:path";
import { atomicWrite } from "./atomic.ts";
import { legacyUserDataPath, userDataLabel, userDataPath } from "./user-data.ts";

/** Keys this session was given but not asked to keep. Dies with the window. */
const held = new Map<string, string>();

export type CredentialSource = "environment" | "session" | "saved";

/** The saved file, read once. `undefined` until the first look at it. */
let saved: Record<string, string> | undefined;

export function keyFor(name: string): string | undefined {
  return use(process.env[name]) ?? use(held.get(name)) ?? use(fromDisk()[name]);
}

export function credentialSource(name: string): CredentialSource | undefined {
  if (use(process.env[name]) !== undefined) return "environment";
  if (use(held.get(name)) !== undefined) return "session";
  return use(fromDisk()[name]) === undefined ? undefined : "saved";
}

/** Values that must never survive in shell output, regardless of their source. */
export function credentialValues(): string[] {
  const stored = fromDisk();
  // A second process can replace the saved store after this session cached it.
  // Redact both snapshots so neither a stale nor a newly persisted key can
  // cross the shell boundary.
  const current = readSavedStore();
  const names = new Set([
    ...held.keys(),
    ...Object.keys(stored),
    ...Object.keys(current),
  ]);
  const values = new Set([
    ...held.values(),
    ...Object.values(stored),
    ...Object.values(current),
  ]);
  for (const name of names) {
    const environment = use(process.env[name]);
    if (environment !== undefined) values.add(environment);
  }
  return [...values].filter((value) => use(value) !== undefined);
}

export function hasSaved(name: string): boolean {
  return use(fromDisk()[name]) !== undefined;
}

/** Take a key for this session only. Nothing is written anywhere. */
export function hold(name: string, value: string): void {
  held.set(name, value);
}

/**
 * Take a key and write it down, returning the path it went to.
 *
 * Owner-only permissions on a directory that did not exist a moment ago: the
 * mode is set at creation rather than fixed afterwards, because between the
 * two there is a window where the file is readable and the key is in it.
 * Windows ignores the mode and relies on the profile directory's own ACL.
 */
export async function keep(name: string, value: string): Promise<string> {
  const file = storePath();
  const all = { ...fromDisk(), [name]: value };
  await persist(file, all);
  hold(name, value);
  saved = all;

  return file;
}

/** Remove only the saved copy. An environment or session value is untouched. */
export async function forgetSaved(name: string): Promise<boolean> {
  const all = { ...fromDisk() };
  if (use(all[name]) === undefined) return false;
  delete all[name];
  const file = storePath();
  await persist(file, all);
  saved = all;
  return true;
}

export function storePath(): string {
  return userDataPath("credentials.json");
}

/**
 * The store path, written the way the user would write it.
 *
 * This label is compact enough for both menu rows and transient feedback. The
 * canonical path remains discoverable without pushing useful copy off-screen.
 */
export function storeLabel(): string {
  return userDataLabel("credentials.json");
}

/** Forget what was read, so the next look goes back to disk. For tests. */
export function reload(): void {
  saved = undefined;
  held.clear();
}

function fromDisk(): Record<string, string> {
  if (saved !== undefined) return saved;
  saved = readSavedStore();
  return saved;
}

function readSavedStore(): Record<string, string> {
  const current = readStore(storePath());
  const legacy = legacyUserDataPath("credentials.json");
  return current ?? (legacy === undefined ? undefined : readStore(legacy)) ?? {};
}

function readStore(file: string): Record<string, string> | undefined {
  try {
    const parsed = JSON.parse(readFileSync(file, "utf8")) as unknown;
    // Anything that is not a string is not a key, whatever the file says.
    return Object.fromEntries(
      Object.entries(parsed as Record<string, unknown>).filter(
        (entry): entry is [string, string] => typeof entry[1] === "string",
      ),
    );
  } catch (error) {
    // Only a missing canonical file falls through to the legacy location. A
    // malformed new store must not resurrect an older credential silently.
    return (error as NodeJS.ErrnoException).code === "ENOENT" ? undefined : {};
  }
}

async function persist(file: string, values: Record<string, string>): Promise<void> {
  const directory = path.dirname(file);
  await mkdir(directory, { recursive: true, mode: 0o700 });
  if (process.platform !== "win32") await chmod(directory, 0o700);
  await atomicWrite(file, `${JSON.stringify(values, null, 2)}\n`, { mode: 0o600 });
}

/** An empty variable is an unset variable — an exported "" is not a key. */
function use(value: string | undefined): string | undefined {
  return value === undefined || value === "" ? undefined : value;
}
