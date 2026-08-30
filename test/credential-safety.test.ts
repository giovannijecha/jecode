import { test } from "node:test";
import assert from "node:assert/strict";
import {
  credentialRedactor,
  redactCredentials,
  shellEnvironment,
} from "../src/credential-safety.ts";

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

test("withholds common credential names with compact or registry-specific syntax", () => {
  const environment = shellEnvironment({
    PATH: "fixture-path",
    GITHUB_PAT: "fixture-github-pat",
    PGPASSWORD: "fixture-postgres-password",
    MYSQL_PWD: "fixture-mysql-password",
    "npm_config_//registry.npmjs.org/:_authToken": "fixture-npm-token",
  });

  assert.deepEqual(environment, { PATH: "fixture-path" });
});

test("redacts credentials discovered through normalized environment names", () => {
  const source = {
    GITHUB_PAT: "fixture-github-pat",
    "npm_config_//registry.npmjs.org/:_authToken": "fixture-npm-token",
  };

  assert.equal(
    redactCredentials("fixture-github-pat and fixture-npm-token", source),
    "[credential redacted] and [credential redacted]",
  );
});
