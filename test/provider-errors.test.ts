import { test } from "node:test";
import assert from "node:assert/strict";
import { providerFailure } from "../src/provider-errors.ts";
import {
  isRetryableGenerationFailure,
  normalizeProviderError,
  providerFailureDetails,
} from "../src/providers/failure.ts";
import type { HttpError } from "../src/providers/http.ts";
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
    "Anthropic API: https://provider.example.test/v1/models -> 400 Bad Request · requested model is unavailable",
  );
  assert.equal(
    providerFailure(provider("openai-codex"), error, true).startsWith("OpenAI Account: "),
    true,
  );
  assert.equal(providerFailure(provider("ollama"), error, true).startsWith("Ollama API: "), true);
});

test("provider failures distinguish transient rate limits from hard quota and billing stops", () => {
  const rate = Object.assign(
    new Error("https://api.openai.com/v1/responses -> 429 Too Many Requests"),
    {
      status: 429,
      body: JSON.stringify({ error: { code: "rate_limit_exceeded", message: "tokens per min" } }),
      retryAfterMs: 250,
    },
  ) as HttpError;
  const quota = Object.assign(new Error("429 Too Many Requests"), {
    status: 429,
    body: JSON.stringify({
      error: {
        code: "insufficient_quota",
        message: "You exceeded your quota. Check your plan and billing details.",
      },
    }),
    retryAfterMs: 250,
  }) as HttpError;
  const billing = Object.assign(new Error("openai stream error: You have no credits remaining"), {
    code: "billing_hard_limit_reached",
  });
  const spendLimit = Object.assign(new Error("anthropic stream error: limit reached"), {
    type: "enforced_spend_limit_reached",
  });

  assert.equal(providerFailureDetails("openai", rate).kind, "rate-limit");
  assert.equal(isRetryableGenerationFailure("openai", rate), true);
  assert.equal(providerFailureDetails("openai", quota).kind, "quota");
  assert.equal(isRetryableGenerationFailure("openai", quota), false);
  assert.equal(providerFailureDetails("openai", billing).kind, "billing");
  assert.equal(providerFailure(provider(), billing, true),
    "OpenAI API: credits exhausted · add credits or choose another provider in /models");
  assert.equal(providerFailure(provider("anthropic"), spendLimit, true),
    "Anthropic API: billing limit reached · check provider billing or choose another provider in /models");
});

test("provider failures expose only a validated support request identifier", () => {
  const error = Object.assign(new Error("service unavailable"), {
    status: 503,
    requestId: "req_fixture-123",
  });
  assert.equal(
    providerFailure(provider(), error, true),
    "OpenAI API: temporarily unavailable · retry later · request req_fixture-123",
  );

  error.requestId = "unsafe request\nsecret";
  assert.equal(
    providerFailure(provider(), error, true),
    "OpenAI API: temporarily unavailable · retry later",
  );
});

test("the normalized provider contract retains safe diagnostics without exposing headers", () => {
  const source = Object.assign(new Error("service unavailable"), {
    status: 503,
    body: '{"error":{"type":"overloaded_error","message":"busy"}}',
    requestId: "req_fixture-123",
  });
  const error = normalizeProviderError("anthropic", source);

  assert.equal(error.providerId, "anthropic");
  assert.equal(error.kind, "overload");
  assert.equal(error.status, 503);
  assert.equal(error.requestId, "req_fixture-123");
  assert.equal(error.body, source.body);
});

test("provider reasons accept common JSON and plain-text shapes", () => {
  const base = "https://provider.example.test -> 418 Unknown";
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
      httpError("https://provider.example.test -> 418 Unknown", JSON.stringify({ error: unsafe })),
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
