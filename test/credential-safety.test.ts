import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import {
  accountsPath,
  reloadAccounts,
  updateOpenAICodexAccount,
} from "../src/accounts.ts";
import {
  hold,
  keep,
  reload as reloadCredentials,
  storePath,
} from "../src/credentials.ts";
import {
  credentialRedactor,
  MAX_REDACTION_SECRETS,
  redactCredentials,
  shellEnvironment,
} from "../src/credential-safety.ts";
import { USER_STORE_LIMITS } from "../src/user-store.ts";

test("redacts one credential split across stream chunks", () => {
  const redact = credentialRedactor({ OPENAI_API_KEY: "fixture-split-secret" });

  assert.equal(redact.write("before fixture-split-"), "before ");
  const remainder = `${redact.write("secret after")}${redact.end()}`;
  assert.equal(remainder, "[credential redacted] after");
});

test("does not expose the suffix of a longer credential sharing a prefix", () => {
  const redact = credentialRedactor({
    OPENAI_API_KEY: "fixture-prefix",
    ANTHROPIC_API_KEY: "fixture-prefix-secret",
  });

  assert.equal(redact.write("before fixture-prefix"), "before ");
  assert.equal(
    `${redact.write("-secret after")}${redact.end()}`,
    "[credential redacted] after",
  );
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

test("redaction fails closed when the supported secret set is exceeded", () => {
  const source = Object.fromEntries(Array.from(
    { length: MAX_REDACTION_SECRETS + 1 },
    (_, index) => [`TOKEN_${index}`, `fixture-secret-${index}`],
  ));
  const redact = credentialRedactor(source);

  assert.equal(redactCredentials("ordinary output", source), "[credential redacted]");
  assert.equal(redact.write("first chunk"), "[credential redacted]");
  assert.equal(redact.write("second chunk"), "");
  assert.equal(redact.end(), "");
});

test("redaction fails closed for a secret too large to match incrementally", () => {
  const source = { OPENAI_API_KEY: "x".repeat(USER_STORE_LIMITS.accountToken + 1) };
  assert.equal(redactCredentials("ordinary output", source), "[credential redacted]");
});

test("redaction handles 30,000 non-matching characters at the supported secret limit", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-redact-bounded-"));
  const before = process.env["JECODE_HOME"];
  process.env["JECODE_HOME"] = directory;
  reloadAccounts();
  reloadCredentials();
  try {
    const source = Object.fromEntries(Array.from(
      { length: MAX_REDACTION_SECRETS },
      (_, index) => [`TOKEN_${index}`, `fixture-secret-${index}`],
    ));
    const output = "x".repeat(30_000);
    const redact = credentialRedactor(source);

    assert.equal(`${redact.write(output)}${redact.end()}`, output);
  } finally {
    if (before === undefined) delete process.env["JECODE_HOME"];
    else process.env["JECODE_HOME"] = before;
    reloadAccounts();
    reloadCredentials();
    await rm(directory, { recursive: true, force: true });
  }
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

test("redacts API and OAuth credentials replaced by another process", async () => {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-redact-rotated-"));
  const before = process.env["JECODE_HOME"];
  process.env["JECODE_HOME"] = directory;
  reloadAccounts();
  reloadCredentials();
  try {
    await keep("OPENAI_API_KEY", "fixture-api-old");
    await updateOpenAICodexAccount(async () => ({
      accessToken: "fixture-oauth-access-old",
      refreshToken: "fixture-oauth-refresh-old",
      expiresAt: 2_000_000_000_000,
      accountId: "account-1",
    }));

    // Stand in for another Jecode process replacing both stores after this
    // process populated its caches.
    await writeFile(storePath(), JSON.stringify({ OPENAI_API_KEY: "fixture-api-new" }), "utf8");
    await writeFile(accountsPath(), JSON.stringify({
      version: 1,
      accounts: {
        "openai-codex": {
          accessToken: "fixture-oauth-access-new",
          refreshToken: "fixture-oauth-refresh-new",
          expiresAt: 2_000_000_000_001,
          accountId: "account-1",
        },
      },
    }), "utf8");

    assert.equal(
      redactCredentials(
        "fixture-api-old fixture-api-new fixture-oauth-access-old " +
          "fixture-oauth-access-new fixture-oauth-refresh-old fixture-oauth-refresh-new",
        {},
      ),
      "[credential redacted] ".repeat(5) + "[credential redacted]",
    );
  } finally {
    if (before === undefined) delete process.env["JECODE_HOME"];
    else process.env["JECODE_HOME"] = before;
    reloadAccounts();
    reloadCredentials();
    await rm(directory, { recursive: true, force: true });
  }
});
