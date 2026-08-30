// Asking a provider what it will answer to.
//
// Nothing here is a list of models. A catalogue written into this repo is
// wrong the week after it is written, and wrong in the worst way — it names
// models that no longer exist and hides the ones that do. So the question goes
// to the provider, every time the user opens the menu.
//
// All three happen to answer the same GET at `/v1/models` with the same shape,
// `{ data: [{ id }] }`, so the asking is written once and a provider supplies
// only its URL and its headers.
import { getJson } from "./http.js";
export const MAX_MODEL_CATALOG_ENTRIES = 1_000;
export const MAX_MODEL_CATALOG_ITEMS = 4_000;
export const MAX_MODEL_ID_CHARS = 256;
export async function listModels(url, headers, signal, onStatus) {
    const body = await getJson(url, headers, signal, onStatus);
    const data = body.data;
    if (!Array.isArray(data))
        throw new Error(`${url} did not return a model list`);
    const models = [];
    const unique = new Set();
    const inspected = Math.min(data.length, MAX_MODEL_CATALOG_ITEMS);
    for (let index = 0; index < inspected && models.length < MAX_MODEL_CATALOG_ENTRIES; index++) {
        const entry = data[index];
        const id = typeof entry === "object" && entry !== null
            ? entry.id
            : undefined;
        if (typeof id !== "string" ||
            id === "" ||
            id.length > MAX_MODEL_ID_CHARS ||
            unique.has(id))
            continue;
        unique.add(id);
        models.push(id);
    }
    return models;
}
