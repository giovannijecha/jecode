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
import { atomicWrite } from "./atomic.js";
import { legacyUserDataPath, userDataLabel, userDataPath } from "./user-data.js";
/** Keys this session was given but not asked to keep. Dies with the window. */
const held = new Map();
/** The saved file, read once. `undefined` until the first look at it. */
let saved;
export function keyFor(name) {
    return use(process.env[name]) ?? use(held.get(name)) ?? use(fromDisk()[name]);
}
export function credentialSource(name) {
    if (use(process.env[name]) !== undefined)
        return "environment";
    if (use(held.get(name)) !== undefined)
        return "session";
    return use(fromDisk()[name]) === undefined ? undefined : "saved";
}
export function hasSaved(name) {
    return use(fromDisk()[name]) !== undefined;
}
/** Take a key for this session only. Nothing is written anywhere. */
export function hold(name, value) {
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
export async function keep(name, value) {
    const file = storePath();
    const all = { ...fromDisk(), [name]: value };
    await persist(file, all);
    hold(name, value);
    saved = all;
    return file;
}
/** Remove only the saved copy. An environment or session value is untouched. */
export async function forgetSaved(name) {
    const all = { ...fromDisk() };
    if (use(all[name]) === undefined)
        return false;
    delete all[name];
    const file = storePath();
    await persist(file, all);
    saved = all;
    return true;
}
export function storePath() {
    return userDataPath("credentials.json");
}
/**
 * The store path, written the way the user would write it.
 *
 * This label is compact enough for both menu rows and transient feedback. The
 * canonical path remains discoverable without pushing useful copy off-screen.
 */
export function storeLabel() {
    return userDataLabel("credentials.json");
}
/** Forget what was read, so the next look goes back to disk. For tests. */
export function reload() {
    saved = undefined;
    held.clear();
}
function fromDisk() {
    if (saved !== undefined)
        return saved;
    const current = readStore(storePath());
    const legacy = legacyUserDataPath("credentials.json");
    saved = current ?? (legacy === undefined ? undefined : readStore(legacy)) ?? {};
    return saved;
}
function readStore(file) {
    try {
        const parsed = JSON.parse(readFileSync(file, "utf8"));
        // Anything that is not a string is not a key, whatever the file says.
        return Object.fromEntries(Object.entries(parsed).filter((entry) => typeof entry[1] === "string"));
    }
    catch (error) {
        // Only a missing canonical file falls through to the legacy location. A
        // malformed new store must not resurrect an older credential silently.
        return error.code === "ENOENT" ? undefined : {};
    }
}
async function persist(file, values) {
    const directory = path.dirname(file);
    await mkdir(directory, { recursive: true, mode: 0o700 });
    if (process.platform !== "win32")
        await chmod(directory, 0o700);
    await atomicWrite(file, `${JSON.stringify(values, null, 2)}\n`, 0o600);
}
/** An empty variable is an unset variable — an exported "" is not a key. */
function use(value) {
    return value === undefined || value === "" ? undefined : value;
}
