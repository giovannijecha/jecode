// Persistent, non-secret defaults for interactive and batch sessions.
import { readFileSync } from "node:fs";
import { chmod, mkdir } from "node:fs/promises";
import * as path from "node:path";
import { atomicWrite } from "./atomic.js";
import { providerNames } from "./providers/index.js";
import { userDataLabel, userDataPath } from "./user-data.js";
export const EFFORTS = ["low", "medium", "high", "xhigh", "max"];
let saved;
export function readSettings() {
    if (saved === undefined)
        saved = readStore();
    return copy(saved);
}
export async function updateSettings(patch) {
    const next = normalize({ ...readSettings(), ...patch });
    const file = settingsPath();
    const directory = path.dirname(file);
    await mkdir(directory, { recursive: true, mode: 0o700 });
    if (process.platform !== "win32")
        await chmod(directory, 0o700);
    await atomicWrite(file, `${JSON.stringify(next, null, 2)}\n`, { mode: 0o600 });
    saved = next;
    return file;
}
export function settingsPath() {
    return userDataPath("settings.json");
}
export function settingsLabel() {
    return userDataLabel("settings.json");
}
/** Forget the cached read so tests and explicit reloads see the disk again. */
export function reloadSettings() {
    saved = undefined;
}
function readStore() {
    try {
        return normalize(JSON.parse(readFileSync(settingsPath(), "utf8")));
    }
    catch {
        // Missing, unreadable, and malformed stores all fall back safely. A bad
        // preference must never prevent the agent from starting.
        return {};
    }
}
function normalize(value) {
    if (!record(value))
        return {};
    const providers = providerNames();
    const provider = member(value["provider"], providers);
    const models = modelsOf(value["models"], providers);
    const effort = member(value["effort"], EFFORTS);
    const reducedMotion = typeof value["reducedMotion"] === "boolean" ? value["reducedMotion"] : undefined;
    const maxTokens = positiveInteger(value["maxTokens"]);
    const maxSteps = positiveInteger(value["maxSteps"]);
    return {
        ...(provider === undefined ? {} : { provider }),
        ...(models === undefined ? {} : { models }),
        ...(effort === undefined ? {} : { effort }),
        ...(reducedMotion === undefined ? {} : { reducedMotion }),
        ...(maxTokens === undefined ? {} : { maxTokens }),
        ...(maxSteps === undefined ? {} : { maxSteps }),
    };
}
function modelsOf(value, providers) {
    if (!record(value))
        return undefined;
    const models = Object.fromEntries(Object.entries(value).filter((entry) => providers.includes(entry[0]) && typeof entry[1] === "string" && entry[1].trim() !== ""));
    return Object.keys(models).length === 0 ? undefined : models;
}
function member(value, values) {
    return typeof value === "string" && values.includes(value) ? value : undefined;
}
function positiveInteger(value) {
    return typeof value === "number" && Number.isSafeInteger(value) && value > 0 ? value : undefined;
}
function record(value) {
    return typeof value === "object" && value !== null && !Array.isArray(value);
}
function copy(value) {
    return {
        ...value,
        ...(value.models === undefined ? {} : { models: { ...value.models } }),
    };
}
