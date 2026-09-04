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

import { getJson } from "./http.ts";
import type { HttpRetryPolicy } from "./http.ts";

export const MAX_MODEL_CATALOG_ENTRIES = 1_000;
export const MAX_MODEL_CATALOG_ITEMS = 4_000;
export const MAX_MODEL_ID_CHARS = 256;

export type ModelCatalogEntry = Readonly<{
  id: string;
  metadata: Record<string, unknown>;
}>;

export async function listModels(
  url: string,
  headers: Record<string, string>,
  signal?: AbortSignal,
  onStatus?: (status: string) => void,
  retry?: HttpRetryPolicy,
): Promise<string[]> {
  return (await modelCatalog(url, headers, signal, onStatus, retry)).map((entry) => entry.id);
}

export async function modelCatalog(
  url: string,
  headers: Record<string, string>,
  signal?: AbortSignal,
  onStatus?: (status: string) => void,
  retry?: HttpRetryPolicy,
): Promise<ModelCatalogEntry[]> {
  const body = await getJson(url, headers, signal, onStatus, retry);
  const data = (body as { data?: unknown }).data;
  if (!Array.isArray(data)) throw new Error(`${url} did not return a model list`);

  const models: ModelCatalogEntry[] = [];
  const unique = new Set<string>();
  const inspected = Math.min(data.length, MAX_MODEL_CATALOG_ITEMS);

  for (let index = 0; index < inspected && models.length < MAX_MODEL_CATALOG_ENTRIES; index++) {
    const entry = data[index];
    const metadata = record(entry) ? entry : undefined;
    if (metadata === undefined) continue;
    const id = metadata["id"];
    if (
      typeof id !== "string" ||
      id === "" ||
      id.length > MAX_MODEL_ID_CHARS ||
      unique.has(id)
    ) continue;
    unique.add(id);
    models.push({ id, metadata });
  }

  return models;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
