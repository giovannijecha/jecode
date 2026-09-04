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

import * as path from "node:path";
import { atomicWrite } from "./atomic.ts";
import {
  assertDirectoryAnchor,
  captureDirectDirectorySync,
  preparePrivateDirectory,
} from "./directory-anchor.ts";
import type { DirectoryAnchor } from "./directory-anchor.ts";
import { withStoreLock } from "./store-lock.ts";
import { legacyUserDataPath, userDataLabel, userDataPath } from "./user-data.ts";
import {
  assertStoreText,
  readBoundedJsonForMutationSync,
  readBoundedJsonSync,
  USER_STORE_LIMITS,
} from "./user-store.ts";

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
  assertCredential(name, value);
  if (!held.has(name) && held.size >= USER_STORE_LIMITS.credentialEntries) {
    throw new Error("too many session credentials");
  }
  held.set(name, value);
}

/** Remove only the value held by this process. Saved and environment values remain. */
export function forgetSession(name: string): boolean {
  return held.delete(name);
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
  assertCredential(name, value);
  const file = storePath();
  const directory = await prepare(file);
  const anchoredFile = path.join(directory.path, path.basename(file));
  return withStoreLock(anchoredFile, async () => {
    const all = { ...readSavedStoreForMutation(anchoredFile, directory), [name]: value };
    if (Object.keys(all).length > USER_STORE_LIMITS.credentialEntries) {
      throw new Error("too many saved credentials");
    }
    await persist(anchoredFile, all, directory);
    saved = all;
    // A newly saved replacement must become active immediately. Otherwise an
    // older session-only value would keep shadowing the value just written.
    held.delete(name);
    return file;
  }, undefined, async () => assertDirectoryAnchor(directory));
}

/** Remove only the saved copy. An environment or session value is untouched. */
export async function forgetSaved(name: string): Promise<boolean> {
  const file = storePath();
  const directory = await prepare(file);
  const anchoredFile = path.join(directory.path, path.basename(file));
  return withStoreLock(anchoredFile, async () => {
    const all = { ...readSavedStoreForMutation(anchoredFile, directory) };
    if (use(all[name]) === undefined) {
      saved = all;
      return false;
    }
    delete all[name];
    await persist(anchoredFile, all, directory);
    saved = all;
    return true;
  }, undefined, async () => assertDirectoryAnchor(directory));
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

function readSavedStoreForMutation(
  file: string,
  directory: DirectoryAnchor,
): Record<string, string> {
  const current = readCredentialStoreForMutation(file, directory);
  if (current !== undefined) return current;
  const legacy = legacyUserDataPath("credentials.json");
  if (legacy === undefined) return {};
  return readCredentialStoreForMutation(legacy) ?? {};
}

function readCredentialStoreForMutation(
  file: string,
  directory?: DirectoryAnchor,
): Record<string, string> | undefined {
  let anchor = directory;
  if (anchor === undefined) {
    try {
      anchor = captureDirectDirectorySync(path.dirname(file), "credential store directory");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
      throw new Error("credential store is invalid, unsafe, or too large", { cause: error });
    }
  }
  const anchoredFile = path.join(anchor.path, path.basename(file));
  const value = readBoundedJsonForMutationSync(
    anchoredFile,
    USER_STORE_LIMITS.credentialsBytes,
    "credential store",
    anchor,
  );
  if (value === undefined) return undefined;
  if (!record(value)) throw new Error("credential store has an unsupported structure");
  const entries = Object.entries(value);
  if (
    entries.length > USER_STORE_LIMITS.credentialEntries ||
    entries.some(([name, candidate]) => !credential(name, candidate))
  ) throw new Error("credential store has invalid entries");
  return Object.fromEntries(entries) as Record<string, string>;
}

function readStore(file: string): Record<string, string> | undefined {
  try {
    const directory = captureDirectDirectorySync(
      path.dirname(file),
      "credential store directory",
    );
    const anchoredFile = path.join(directory.path, path.basename(file));
    const parsed = readBoundedJsonSync(
      anchoredFile,
      USER_STORE_LIMITS.credentialsBytes,
      directory,
    );
    if (!record(parsed)) return {};
    // Anything that is not a string is not a key, whatever the file says.
    const entries = Object.entries(parsed);
    if (entries.length > USER_STORE_LIMITS.credentialEntries) return {};
    return Object.fromEntries(entries.filter(
      (entry): entry is [string, string] => credential(entry[0], entry[1]),
    ));
  } catch (error) {
    // Only a missing canonical file falls through to the legacy location. A
    // malformed new store must not resurrect an older credential silently.
    return (error as NodeJS.ErrnoException).code === "ENOENT" ? undefined : {};
  }
}

async function prepare(file: string): Promise<DirectoryAnchor> {
  const directory = path.dirname(file);
  return preparePrivateDirectory(directory, "credential store directory");
}

async function persist(
  file: string,
  values: Record<string, string>,
  directory: DirectoryAnchor,
): Promise<void> {
  const text = `${JSON.stringify(values, null, 2)}\n`;
  assertStoreText(text, USER_STORE_LIMITS.credentialsBytes);
  await atomicWrite(file, text, {
    mode: 0o600,
    validate: async () => assertDirectoryAnchor(directory),
  });
}

function assertCredential(name: string, value: string): void {
  if (!credential(name, value)) throw new Error("invalid credential name or value");
}

function credential(name: string, value: unknown): value is string {
  return name.length > 0 && name.length <= USER_STORE_LIMITS.credentialName &&
    /^[A-Za-z_][A-Za-z0-9_]*$/.test(name) &&
    typeof value === "string" && value.length > 0 &&
    value.length <= USER_STORE_LIMITS.credentialValue;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** An empty variable is an unset variable — an exported "" is not a key. */
function use(value: string | undefined): string | undefined {
  return value === undefined || value === "" ? undefined : value;
}
