import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { keep, reload } from "../src/credentials.ts";
import {
  OLLAMA_CLOUD_HOST,
  OLLAMA_LOCAL_HOST,
  parseOllamaEndpoint,
} from "../src/providers/ollama-endpoint.ts";
import {
  configureOllama,
  ollama,
  ollamaConnection,
} from "../src/providers/ollama.ts";

test("infers the local daemon when no Ollama key or host is configured", async () => {
  await inOllamaHome(async () => {
    assert.deepEqual(ollamaConnection(), {
      baseUrl: OLLAMA_LOCAL_HOST,
      loopback: true,
      kind: "local",
      inferred: true,
    });
    assert.equal(ollama.blocked(), undefined);
    assert.equal(ollama.location?.(), "local");
  });
});

test("a saved Ollama key restores the Cloud endpoint without migration", async () => {
  await inOllamaHome(async () => {
    await keep("OLLAMA_API_KEY", "fixture-key");

    assert.deepEqual(ollamaConnection(), {
      baseUrl: OLLAMA_CLOUD_HOST,
      loopback: false,
      kind: "cloud",
      inferred: true,
    });
    assert.equal(ollama.blocked(), undefined);
    assert.equal(ollama.location?.(), "cloud");
  });
});

test("a saved key sends the model request to Ollama Cloud", async (context) => {
  await inOllamaHome(async () => {
    await keep("OLLAMA_API_KEY", "fixture-key");
    const previousFetch = globalThis.fetch;
    let requested = "";
    let authorization = "";
    globalThis.fetch = (async (input, init) => {
      requested = String(input);
      authorization = String((init?.headers as Record<string, string> | undefined)?.authorization ?? "");
      return new Response(JSON.stringify({ data: [{ id: "cloud-model" }] }), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }) as typeof fetch;
    context.after(() => {
      globalThis.fetch = previousFetch;
    });

    assert.deepEqual(await ollama.models(), ["cloud-model"]);
    assert.equal(requested, "https://ollama.com/v1/models");
    assert.equal(authorization, "Bearer fixture-key");
  });
});

test("an explicit session endpoint wins over key-aware inference", async () => {
  await inOllamaHome(async () => {
    await keep("OLLAMA_API_KEY", "fixture-key");
    configureOllama(OLLAMA_LOCAL_HOST);

    assert.equal(ollamaConnection().baseUrl, OLLAMA_LOCAL_HOST);
    assert.equal(ollamaConnection().inferred, false);
    assert.equal(ollama.location?.(), "local");
  });
});

test("an explicit local connection does not send the Cloud key", async (context) => {
  await inOllamaHome(async () => {
    await keep("OLLAMA_API_KEY", "fixture-key");
    configureOllama(OLLAMA_LOCAL_HOST);
    const previousFetch = globalThis.fetch;
    let authorization: string | undefined;
    globalThis.fetch = (async (_input, init) => {
      authorization = (init?.headers as Record<string, string> | undefined)?.authorization;
      return new Response(JSON.stringify({ data: [] }), { status: 200 });
    }) as typeof fetch;
    context.after(() => {
      globalThis.fetch = previousFetch;
    });

    await ollama.models();
    assert.equal(authorization, undefined);
  });
});

test("allows HTTP only for exact loopback endpoints", () => {
  assert.deepEqual(parseOllamaEndpoint("http://127.0.0.1:11434/"), {
    baseUrl: OLLAMA_LOCAL_HOST,
    loopback: true,
  });
  assert.equal(parseOllamaEndpoint("http://[::1]:11434").loopback, true);
  assert.equal(parseOllamaEndpoint("http://localhost:11434").loopback, true);

  assert.throws(
    () => parseOllamaEndpoint("http://models.example.test:11434"),
    /must use HTTPS/,
  );
  assert.throws(
    () => parseOllamaEndpoint("http://127.0.0.1.example.test:11434"),
    /must use HTTPS/,
  );
});

test("normalizes a remote HTTPS base URL", () => {
  assert.deepEqual(parseOllamaEndpoint(" https://models.example.test:443/team/// "), {
    baseUrl: "https://models.example.test/team",
    loopback: false,
  });
});

test("rejects credentials and non-endpoint URL components", () => {
  assert.throws(
    () => parseOllamaEndpoint("https://user:secret@models.example.test"),
    /must not contain credentials/,
  );
  assert.throws(
    () => parseOllamaEndpoint("https://models.example.test?token=value"),
    /query or fragment/,
  );
  assert.throws(() => parseOllamaEndpoint("file:///tmp/ollama"), /HTTP or HTTPS/);
  assert.throws(() => parseOllamaEndpoint("not a URL"), /absolute HTTP\(S\) URL/);
});

test("a configured key never makes remote HTTP usable", async () => {
  await inOllamaHome(async () => {
    await keep("OLLAMA_API_KEY", "fixture-key");
    assert.throws(
      () => configureOllama("http://models.example.test:11434"),
      /must use HTTPS/,
    );
  });
});

async function inOllamaHome(body: () => Promise<void> | void): Promise<void> {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-ollama-"));
  const beforeHome = process.env["JECODE_HOME"];
  const beforeKey = process.env["OLLAMA_API_KEY"];
  process.env["JECODE_HOME"] = directory;
  delete process.env["OLLAMA_API_KEY"];
  reload();
  configureOllama(undefined);
  try {
    await body();
  } finally {
    if (beforeHome === undefined) delete process.env["JECODE_HOME"];
    else process.env["JECODE_HOME"] = beforeHome;
    if (beforeKey === undefined) delete process.env["OLLAMA_API_KEY"];
    else process.env["OLLAMA_API_KEY"] = beforeKey;
    reload();
    configureOllama(undefined);
    await rm(directory, { recursive: true, force: true });
  }
}
