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

export async function listModels(
  url: string,
  headers: Record<string, string>,
  signal?: AbortSignal,
  onStatus?: (status: string) => void,
): Promise<string[]> {
  const body = await getJson(url, headers, signal, onStatus);
  const data = (body as { data?: unknown }).data;
  if (!Array.isArray(data)) throw new Error(`${url} did not return a model list`);

  return data
    .map((entry) => (entry as { id?: unknown }).id)
    .filter((id): id is string => typeof id === "string" && id !== "");
}
