import { test } from "node:test";
import assert from "node:assert/strict";
import { credentialRedactor, shellEnvironment } from "../src/credential-safety.ts";

test("redacts one credential split across stream chunks", () => {
  const redact = credentialRedactor({ OPENAI_API_KEY: "fixture-split-secret" });

  assert.equal(redact.write("before fixture-split-"), "before ");
  assert.equal(redact.write("secret after"), "[credential redacted] after");
  assert.equal(redact.end(), "");
});

test("keeps the streaming buffer bounded for an overlapping credential", () => {
  const redact = credentialRedactor({ JECODE_TEST_KEY: "aa" });

  assert.equal(
    redact.write("aaaaaaaa"),
    "[credential redacted]".repeat(4),
  );
  assert.equal(redact.end(), "");
});

test("preserves ordinary shell configuration", () => {
  const environment = shellEnvironment({
    PATH: "fixture-path",
    JECODE_MODE: "review",
    JECODE_TOKEN: "fixture-token",
  });

  assert.deepEqual(environment, { PATH: "fixture-path", JECODE_MODE: "review" });
});
