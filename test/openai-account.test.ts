import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import * as path from "node:path";
import {
  openAICodexAccount,
  reloadAccounts,
  updateOpenAICodexAccount,
} from "../src/accounts.ts";
import { openAIAuthorization } from "../src/openai-account.ts";

const CLAIMS = "https://api.openai.com/auth";

test("cancelling a caller does not cancel an in-flight refresh-token rotation", async (context) => {
  const directory = await mkdtemp(path.join(tmpdir(), "jecode-refresh-account-"));
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
