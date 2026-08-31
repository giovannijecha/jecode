import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import {
  accountValues,
  accountsLabel,
  accountsPath,
  openAICodexAccount,
  reloadAccounts,
  updateOpenAICodexAccount,
  type OpenAICodexAccount,
} from "../src/accounts.ts";

const ACCOUNT: OpenAICodexAccount = {
  accessToken: "access-token",
  refreshToken: "refresh-token",
  expiresAt: 2_000_000_000_000,
  accountId: "account-1",
};

test("a ChatGPT account survives reload without exposing its tokens in the label", async () => {
  await inStore(async () => {
    await updateOpenAICodexAccount(async () => ({ ...ACCOUNT, email: "person@example.test" }));
    reloadAccounts();

    assert.deepEqual(openAICodexAccount(), { ...ACCOUNT, email: "person@example.test" });
    assert.deepEqual(accountValues(), ["access-token", "refresh-token"]);
    assert.match(accountsLabel(), /accounts\.json$/);
    assert.doesNotMatch(accountsLabel(), /access-token|refresh-token/);
  });
});

test("account mutations serialize so refresh metadata cannot overwrite another process", async () => {
  await inStore(async () => {
    await updateOpenAICodexAccount(async () => ACCOUNT);
    await Promise.all([
      updateOpenAICodexAccount(async (current) => {
        await new Promise((resolve) => setTimeout(resolve, 20));
        return { ...(current as OpenAICodexAccount), email: "person@example.test" };
      }),
      updateOpenAICodexAccount(async (current) => ({
        ...(current as OpenAICodexAccount),
        plan: "plus",
      })),
    ]);
    reloadAccounts();

    assert.equal(openAICodexAccount()?.email, "person@example.test");
    assert.equal(openAICodexAccount()?.plan, "plus");
  });
});

test("a malformed or incomplete account store is treated as disconnected", async () => {
  await inStore(async () => {
    await writeFile(accountsPath(), JSON.stringify({ version: 1, accounts: {
      "openai-codex": { accessToken: "partial" },
    } }), "utf8");
    reloadAccounts();

    assert.equal(openAICodexAccount(), undefined);
    assert.deepEqual(accountValues(), []);
  });
});

test("removing an account leaves no usable OAuth credential", async () => {
  await inStore(async () => {
    await updateOpenAICodexAccount(async () => ACCOUNT);
    await updateOpenAICodexAccount(async () => undefined);
    reloadAccounts();

    assert.equal(openAICodexAccount(), undefined);
    const parsed = JSON.parse(await readFile(accountsPath(), "utf8")) as { accounts: object };
    assert.deepEqual(parsed.accounts, {});
  });
});

test("an already-cancelled account mutation never writes OAuth state", async () => {
  await inStore(async () => {
    const control = new AbortController();
    control.abort(new Error("stop"));

    await assert.rejects(
      updateOpenAICodexAccount(async () => ACCOUNT, control.signal),
      /stop/,
    );
    assert.equal(openAICodexAccount(), undefined);
  });
});

test("the OAuth account file is owner-only on POSIX", { skip: process.platform === "win32" }, async () => {
  await inStore(async () => {
    await updateOpenAICodexAccount(async () => ACCOUNT);
    assert.equal((await stat(accountsPath())).mode & 0o777, 0o600);
    assert.equal((await stat(path.dirname(accountsPath()))).mode & 0o777, 0o700);
  });
});

async function inStore(body: () => Promise<void>): Promise<void> {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-accounts-"));
  const before = process.env["JECODE_HOME"];
  process.env["JECODE_HOME"] = directory;
  reloadAccounts();
  try {
    await body();
  } finally {
    if (before === undefined) delete process.env["JECODE_HOME"];
    else process.env["JECODE_HOME"] = before;
    reloadAccounts();
    await rm(directory, { recursive: true, force: true });
  }
}
