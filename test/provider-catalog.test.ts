import { test } from "node:test";
import assert from "node:assert/strict";
import {
  listModels,
  MAX_MODEL_CATALOG_ENTRIES,
  MAX_MODEL_CATALOG_ITEMS,
  MAX_MODEL_ID_CHARS,
} from "../src/providers/catalog.ts";

test("a model list keeps the ids and drops everything else", async () => {
  const body = { data: [{ id: "one" }, { id: 7 }, null, 4, {}, { id: "two" }] };
  const original = globalThis.fetch;
  globalThis.fetch = (() =>
    Promise.resolve(new Response(JSON.stringify(body), { status: 200 }))) as typeof fetch;

  try {
    assert.deepEqual(await listModels("https://example.test/v1/models", {}), ["one", "two"]);
  } finally {
    globalThis.fetch = original;
  }
});

test("a model catalogue is deduplicated and bounded", async () => {
  const body = {
    data: [
      { id: "same" },
      { id: "same" },
      { id: "x".repeat(MAX_MODEL_ID_CHARS + 1) },
      ...Array.from(
        { length: MAX_MODEL_CATALOG_ENTRIES + 10 },
        (_, index) => ({ id: `model-${index}` }),
      ),
    ],
  };
  const original = globalThis.fetch;
  globalThis.fetch = (() =>
    Promise.resolve(new Response(JSON.stringify(body), { status: 200 }))) as typeof fetch;

  try {
    const models = await listModels("https://example.test/v1/models", {});
    assert.equal(models.length, MAX_MODEL_CATALOG_ENTRIES);
    assert.equal(models[0], "same");
    assert.equal(new Set(models).size, models.length);
    assert.equal(models.some((id) => id.length > MAX_MODEL_ID_CHARS), false);
  } finally {
    globalThis.fetch = original;
  }
});

test("a model catalogue stops inspecting an oversized invalid list", async () => {
  const body = {
    data: [
      ...Array.from({ length: MAX_MODEL_CATALOG_ITEMS }, () => ({})),
      { id: "outside-the-scan-budget" },
    ],
  };
  const original = globalThis.fetch;
  globalThis.fetch = (() =>
    Promise.resolve(new Response(JSON.stringify(body), { status: 200 }))) as typeof fetch;

  try {
    assert.deepEqual(await listModels("https://example.test/v1/models", {}), []);
  } finally {
    globalThis.fetch = original;
  }
});
