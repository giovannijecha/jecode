import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { keep, reload } from "../src/credentials.ts";
import { ollama } from "../src/providers/ollama.ts";

test("Ollama requires an API key before catalogue, metadata, or generation requests", async () => {
  await inOllamaHome(async () => {
    globalThis.fetch = async () => { assert.fail("missing authentication must not reach the network"); };
    assert.match(ollama.blocked() ?? "", /OLLAMA_API_KEY is not set/);
    await assert.rejects(ollama.models(), /OLLAMA_API_KEY is not set/);
    await assert.rejects(ollama.contextWindow!("fixture"), /OLLAMA_API_KEY is not set/);
    await assert.rejects(ollama.send({
      model: "fixture", system: "", messages: [], tools: [], maxTokens: 100, effort: "low",
    }), /OLLAMA_API_KEY is not set/);
  });
});

test("a saved key loads the cloud catalogue without an endpoint setting", async () => {
  await inOllamaHome(async () => {
    await keep("OLLAMA_API_KEY", "fixture-key");
    globalThis.fetch = async (input, init) => {
      assert.equal(String(input), "https://ollama.com/v1/models");
      assert.equal((init?.headers as Record<string, string>).authorization, "Bearer fixture-key");
      return json({ data: [{ id: "cloud-model" }] });
    };
    assert.equal(ollama.blocked(), undefined);
    assert.deepEqual(await ollama.models(), ["cloud-model"]);
  });
});

test("cloud context uses model metadata with headroom and never probes loaded local models", async () => {
  await inOllamaHome(async () => {
    process.env["OLLAMA_API_KEY"] = "fixture-key";
    for (const tokens of [4_096, 131_072]) {
      const model = `cloud-capacity-${tokens}`;
      let requests = 0;
      globalThis.fetch = async (input, init) => {
        requests++;
        assert.equal(String(input), "https://ollama.com/api/show");
        assert.deepEqual(JSON.parse(String(init?.body)), { model });
        assert.equal((init?.headers as Record<string, string>).authorization, "Bearer fixture-key");
        return json({ model_info: { "fixture.context_length": tokens } });
      };
      const expected = { tokens: Math.floor(tokens * 95 / 100) };
      assert.deepEqual(await ollama.contextWindow!(model), expected);
      assert.deepEqual(await ollama.contextWindow!(model), expected);
      assert.equal(requests, 1, "fresh cloud metadata is reused");
    }
  });
});

test("expired cloud context metadata is refreshed", async () => {
  await inOllamaHome(async () => {
    process.env["OLLAMA_API_KEY"] = "fixture-key";
    const realNow = Date.now;
    let now = realNow();
    let requests = 0;
    Date.now = () => now;
    try {
      globalThis.fetch = async () => {
        requests++;
        return json({ model_info: { "fixture.context_length": requests === 1 ? 131_072 : 65_536 } });
      };
      assert.deepEqual(await ollama.contextWindow!("cloud-expiry"), { tokens: 124_518 });
      now += 30_001;
      assert.deepEqual(await ollama.contextWindow!("cloud-expiry"), { tokens: 62_259 });
      assert.equal(requests, 2);
    } finally { Date.now = realNow; }
  });
});

test("unavailable or invalid cloud metadata leaves budgeting to the safe fallback", async () => {
  await inOllamaHome(async () => {
    process.env["OLLAMA_API_KEY"] = "fixture-key";
    const values = [null, {}, { model_info: [] }, ...[0, 4_095, 10_000_001, "131072", 4_096.5]
      .map((count) => ({ model_info: { "fixture.context_length": count } }))];
    for (const [index, value] of values.entries()) {
      globalThis.fetch = async () => json(value);
      assert.equal(await ollama.contextWindow!(`cloud-invalid-${index}`), undefined);
    }
    globalThis.fetch = async () => new Response("missing", { status: 404 });
    assert.equal(await ollama.contextWindow!("cloud-unavailable"), undefined);
  });
});

test("cloud context cancellation propagates even with a cached capacity", async () => {
  await inOllamaHome(async () => {
    process.env["OLLAMA_API_KEY"] = "fixture-key";
    globalThis.fetch = async () => json({ model_info: { "fixture.context_length": 32_768 } });
    await ollama.contextWindow!("cloud-cancel-cached");
    const control = new AbortController();
    control.abort(new Error("context cancelled"));
    globalThis.fetch = async () => { assert.fail("cancelled metadata must not fetch"); };
    await assert.rejects(ollama.contextWindow!("cloud-cancel-cached", control.signal), /context cancelled/);

    const active = new AbortController();
    globalThis.fetch = async () => {
      active.abort(new Error("context interrupted"));
      throw active.signal.reason;
    };
    await assert.rejects(ollama.contextWindow!("cloud-cancel-active", active.signal), /context interrupted/);
  });
});

function json(value: unknown): Response {
  return new Response(JSON.stringify(value), { status: 200, headers: { "content-type": "application/json" } });
}

async function inOllamaHome(body: () => Promise<void>): Promise<void> {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-ollama-"));
  const beforeHome = process.env["JECODE_HOME"];
  const beforeKey = process.env["OLLAMA_API_KEY"];
  const previousFetch = globalThis.fetch;
  process.env["JECODE_HOME"] = directory;
  delete process.env["OLLAMA_API_KEY"];
  reload();
  try {
    await body();
  } finally {
    if (beforeHome === undefined) delete process.env["JECODE_HOME"];
    else process.env["JECODE_HOME"] = beforeHome;
    if (beforeKey === undefined) delete process.env["OLLAMA_API_KEY"];
    else process.env["OLLAMA_API_KEY"] = beforeKey;
    globalThis.fetch = previousFetch;
    reload();
    await rm(directory, { recursive: true, force: true });
  }
}
