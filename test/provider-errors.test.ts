import { test } from "node:test";
import assert from "node:assert/strict";
import { providerFailure } from "../src/provider-errors.ts";
import type { Provider } from "../src/types.ts";

function provider(id = "openai"): Provider {
  return {
    id,
    defaultModel: "fixture-model",
    auth: { kind: "api-key", keyVar: "FIXTURE_API_KEY" },
    blocked: () => undefined,
    models: () => Promise.resolve([]),
    send: () => Promise.reject(new Error("not used")),
  };
}

function httpError(message: string, body: string): Error {
  return Object.assign(new Error(message), { status: 400, body });
}

test("provider failures keep one concise JSON reason across provider labels", () => {
  const error = httpError(
    "https://provider.example.test/v1/models -> 400 Bad Request",
    JSON.stringify({ error: { message: "requested model is unavailable" } }),
  );

  assert.equal(
    providerFailure(provider("anthropic"), error, true),
    "Anthropic: https://provider.example.test/v1/models -> 400 Bad Request · requested model is unavailable",
  );
  assert.equal(
    providerFailure(provider("openai-codex"), error, true).startsWith("ChatGPT: "),
    true,
  );
  assert.equal(providerFailure(provider("ollama"), error, true).startsWith("Ollama: "), true);
});

test("provider reasons accept common JSON and plain-text shapes", () => {
  const base = "https://provider.example.test -> 429 Too Many Requests";
  assert.equal(providerFailure(provider(), httpError(base, '{"error":"slow down"}')), `${base} · slow down`);
  assert.equal(providerFailure(provider(), httpError(base, '{"message":"slow down"}')), `${base} · slow down`);
  assert.equal(providerFailure(provider(), httpError(base, '{"detail":"slow down"}')), `${base} · slow down`);
  assert.equal(providerFailure(provider(), httpError(base, "slow down")), `${base} · slow down`);
});

test("provider reasons omit HTML, malformed JSON, and duplicate messages", () => {
  const base = "https://provider.example.test -> 400 Bad Request";
  assert.equal(providerFailure(provider(), httpError(base, "<html>gateway failed</html>")), base);
  assert.equal(providerFailure(provider(), httpError(base, '{"error":{"message":"cut off"}')), base);
  assert.equal(providerFailure(provider(), httpError(`${base}: invalid model`, "invalid model")), `${base}: invalid model`);
});

test("provider reasons are redacted, terminal-safe, bounded, and grapheme-complete", () => {
  const before = process.env["OPENAI_API_KEY"];
  const secret = "fixture-provider-secret-8173";
  process.env["OPENAI_API_KEY"] = secret;
  try {
    const unsafe = `${secret}\nfailed\u001b[31m\u202e${"x".repeat(600)}😀`;
    const shown = providerFailure(
      provider(),
      httpError("https://provider.example.test -> 401 Unauthorized", JSON.stringify({ error: unsafe })),
    );

    assert.match(shown, /\[credential redacted\] failed␛\[31m\\u202e/);
    assert.doesNotMatch(shown, new RegExp(secret));
    assert.equal(shown.includes("\n"), false);
    assert.equal(shown.isWellFormed(), true);
    assert.ok(shown.length <= 570);
  } finally {
    if (before === undefined) delete process.env["OPENAI_API_KEY"];
    else process.env["OPENAI_API_KEY"] = before;
  }
});
