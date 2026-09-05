import { test, type TestContext } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import {
  accountsPath,
  openAICodexAccount,
  reloadAccounts,
  updateOpenAICodexAccount,
  type OpenAICodexAccount,
} from "../src/accounts.ts";
import { openAIAuthorization } from "../src/openai-account.ts";

const CLAIMS = "https://api.openai.com/auth";

test("a disconnected account names the required sign-in route", async (context) => {
  await accountStore(context, "jecode-missing-account-");
  await assert.rejects(openAIAuthorization(), { message: "OpenAI Account is not connected" });
});

test("cancelling a caller does not cancel an in-flight refresh-token rotation", async (context) => {
  await accountStore(context, "jecode-refresh-account-");

  await updateOpenAICodexAccount(async () => ({
    accessToken: accessToken("old-access"),
    refreshToken: "old-refresh",
    expiresAt: Date.now() - 1,
    accountId: "account-1",
  }));

  let release: ((response: Response) => void) | undefined;
  let markStarted: (() => void) | undefined;
  const started = new Promise<void>((resolve) => { markStarted = resolve; });
  globalThis.fetch = ((_input: string | URL | Request, init?: RequestInit) => new Promise<Response>((resolve, reject) => {
    release = resolve;
    markStarted?.();
    const signal = init?.signal;
    signal?.addEventListener("abort", () => reject(signal.reason), { once: true });
  })) as typeof fetch;

  const control = new AbortController();
  const waiting = openAIAuthorization(undefined, control.signal);
  await started;
  control.abort(new Error("caller interrupted"));
  await assert.rejects(waiting, /caller interrupted/);

  release?.(json({
    access_token: accessToken("new-access"),
    refresh_token: "new-refresh",
    expires_in: 3_600,
  }));

  await waitFor(() => openAICodexAccount()?.refreshToken === "new-refresh");
  assert.equal(openAICodexAccount()?.refreshToken, "new-refresh");
});

test("401 recovery reloads fresh rotations and refreshes expired rotations", async (context) => {
  await accountStore(context, "jecode-reloaded-account-");
  await updateOpenAICodexAccount(async () => account("cached-access", "cached-refresh", 3_600_000));
  await writeAccount(account("rotated-access", "rotated-refresh", 3_600_000));

  let refreshes = 0;
  const statuses: string[] = [];
  globalThis.fetch = (async () => {
    refreshes++;
    return json({});
  }) as typeof fetch;

  const reloaded = await openAIAuthorization(
    "cached-access",
    undefined,
    (status) => statuses.push(status),
  );

  assert.deepEqual(reloaded, {
    accessToken: "rotated-access",
    accountId: "account-1",
  });
  assert.equal(refreshes, 0);
  assert.equal(statuses.length, 0);

  await writeAccount(account("expired-access", "expired-refresh", -1));
  globalThis.fetch = (async () => {
    refreshes++;
    return json({
      access_token: accessToken("refreshed-access"),
      refresh_token: "refreshed-refresh",
      expires_in: 3_600,
    });
  }) as typeof fetch;

  const refreshed = await openAIAuthorization(
    "rotated-access",
    undefined,
    (status) => statuses.push(status),
  );

  assert.deepEqual(refreshed, {
    accessToken: accessToken("refreshed-access"),
    accountId: "account-1",
  });
  assert.equal(refreshes, 1);
  assert.deepEqual(statuses, ["Refreshing OpenAI Account sign-in"]);
  assert.equal(openAICodexAccount()?.refreshToken, "refreshed-refresh");
});

async function accountStore(context: TestContext, prefix: string): Promise<void> {
  const directory = await mkdtemp(path.join(tmpdir(), prefix));
  const beforeHome = process.env["JECODE_HOME"];
  const previousFetch = globalThis.fetch;
  process.env["JECODE_HOME"] = directory;
  reloadAccounts();
  context.after(async () => {
    globalThis.fetch = previousFetch;
    if (beforeHome === undefined) delete process.env["JECODE_HOME"];
    else process.env["JECODE_HOME"] = beforeHome;
    reloadAccounts();
    await rm(directory, { recursive: true, force: true });
  });
}

function account(accessTokenValue: string, refreshToken: string, expiresIn: number): OpenAICodexAccount {
  return {
    accessToken: accessTokenValue,
    refreshToken,
    expiresAt: Date.now() + expiresIn,
    accountId: "account-1",
  };
}

async function writeAccount(value: OpenAICodexAccount): Promise<void> {
  await writeFile(accountsPath(), `${JSON.stringify({
    version: 1,
    accounts: { "openai-codex": value },
  }, null, 2)}\n`, "utf8");
}

async function waitFor(predicate: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt++) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  throw new Error("refresh did not finish");
}

function accessToken(signature: string): string {
  return jwt({ [CLAIMS]: { chatgpt_account_id: "account-1", chatgpt_plan_type: "plus" } }, signature);
}

function jwt(payload: Record<string, unknown>, signature: string): string {
  const header = Buffer.from(JSON.stringify({ alg: "none" })).toString("base64url");
  const body = Buffer.from(JSON.stringify(payload)).toString("base64url");
  return `${header}.${body}.${signature}`;
}

function json(value: unknown): Response {
  return new Response(JSON.stringify(value), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}
