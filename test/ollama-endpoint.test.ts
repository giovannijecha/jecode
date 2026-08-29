import { test } from "node:test";
import assert from "node:assert/strict";
import { parseOllamaEndpoint } from "../src/providers/ollama-endpoint.ts";
import { ollama } from "../src/providers/ollama.ts";

test("allows HTTP only for exact loopback endpoints", () => {
  assert.deepEqual(parseOllamaEndpoint("http://127.0.0.1:11434/"), {
    baseUrl: "http://127.0.0.1:11434",
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

test("a configured key never makes remote HTTP usable", () => {
  const beforeHost = process.env["OLLAMA_HOST"];
  const beforeKey = process.env["OLLAMA_API_KEY"];
  process.env["OLLAMA_HOST"] = "http://models.example.test:11434";
  process.env["OLLAMA_API_KEY"] = "fixture-key";

  try {
    assert.match(ollama.blocked() ?? "", /must use HTTPS/);
    assert.throws(() => ollama.models(), /must use HTTPS/);
  } finally {
    if (beforeHost === undefined) delete process.env["OLLAMA_HOST"];
    else process.env["OLLAMA_HOST"] = beforeHost;
    if (beforeKey === undefined) delete process.env["OLLAMA_API_KEY"];
    else process.env["OLLAMA_API_KEY"] = beforeKey;
  }
});
