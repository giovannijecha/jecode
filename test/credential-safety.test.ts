import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import { reloadAccounts, updateOpenAICodexAccount } from "../src/accounts.ts";
import { hold, reload as reloadCredentials } from "../src/credentials.ts";
import {
  credentialRedactor,
  redactCredentials,
  shellEnvironment,
} from "../src/credential-safety.ts";

test("redacts one credential split across stream chunks", () => {
  const redact = credentialRedactor({ OPENAI_API_KEY: "fixture-split-secret" });

  assert.equal(redact.write("before fixture-split-"), "before ");
  const remainder = `${redact.write("secret after")}${redact.end()}`;
  assert.equal(remainder, "[credential redacted] after");
});

test("keeps the streaming buffer bounded for an overlapping credential", () => {
  const redact = credentialRedactor({ JECODE_TEST_KEY: "aaaaaaaa" });

  assert.equal(
    redact.write("aaaaaaaaaaaaaaaa"),
    "[credential redacted]".repeat(2),
  );
  assert.equal(redact.end(), "");
});

test("does not redact ordinary text that matches a short heuristic secret", () => {
  assert.equal(
    redactCredentials("build 1 of 21", { DEBUG_AUTH: "1" }),
    "build 1 of 21",
  );
});

test("always redacts an explicitly held credential even when it is short", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-redact-short-"));
  const before = process.env["JECODE_HOME"];
  process.env["JECODE_HOME"] = directory;
  reloadCredentials();
  try {
    hold("OPENAI_API_KEY", "1");
    assert.equal(redactCredentials("key=1", {}), "key=[credential redacted]");
  } finally {
    if (before === undefined) delete process.env["JECODE_HOME"];
    else process.env["JECODE_HOME"] = before;
    reloadCredentials();
    await rm(directory, { recursive: true, force: true });
  }
});

test("always redacts a provider API key supplied directly by the environment", () => {
  assert.equal(
    redactCredentials("key=1", { OPENAI_API_KEY: "1" }),
    "key=[credential redacted]",
  );
});

test("preserves ordinary shell configuration", () => {
  const environment = shellEnvironment({
    PATH: "fixture-path",
    JECODE_MODE: "review",
    JECODE_TOKEN: "fixture-token",
  });

  assert.deepEqual(environment, { PATH: "fixture-path", JECODE_MODE: "review" });
});

test("preserves safe agent and workspace paths whose names contain credential words", () => {
  const environment = shellEnvironment({
    SSH_AUTH_SOCK: "/tmp/agent.sock",
    PWD: "/workspace",
    OLDPWD: "/previous",
    PASSWORD_STORE_DIR: "/home/person/.password-store",
  });

  assert.deepEqual(environment, {
    SSH_AUTH_SOCK: "/tmp/agent.sock",
    PWD: "/workspace",
    OLDPWD: "/previous",
    PASSWORD_STORE_DIR: "/home/person/.password-store",
  });
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

test("redacts saved OAuth access and refresh tokens", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-redact-account-"));
  const before = process.env["JECODE_HOME"];
  process.env["JECODE_HOME"] = directory;
  reloadAccounts();
  try {
    await updateOpenAICodexAccount(async () => ({
      accessToken: "fixture-oauth-access",
      refreshToken: "fixture-oauth-refresh",
      expiresAt: 2_000_000_000_000,
      accountId: "account-1",
    }));
    assert.equal(
      redactCredentials("fixture-oauth-access and fixture-oauth-refresh", {}),
      "[credential redacted] and [credential redacted]",
    );
  } finally {
    if (before === undefined) delete process.env["JECODE_HOME"];
    else process.env["JECODE_HOME"] = before;
    reloadAccounts();
    await rm(directory, { recursive: true, force: true });
  }
});
