import { describe, test } from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { Agent, get } from "node:http";
import type { OpenAICodexAccount } from "../src/accounts.ts";
import { oauthRequest } from "../src/oauth-http.ts";
import {
  beginBrowserLogin,
  beginDeviceLogin,
  refreshOpenAITokens,
  revokeOpenAITokens,
} from "../src/openai-oauth.ts";

const CLAIMS = "https://api.openai.com/auth";

describe("OpenAI OAuth protocol", { concurrency: false }, () => {

test("browser sign-in uses PKCE, verifies state, and returns one ChatGPT account", async (context) => {
  const previousFetch = globalThis.fetch;
  let exchangeBody = "";
  globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit) => {
    exchangeBody = String(init?.body ?? "");
    return json(tokens("access-one", "refresh-one"));
  }) as typeof fetch;
  context.after(() => { globalThis.fetch = previousFetch; });

  const login = await beginBrowserLogin();
  const agent = new Agent({ keepAlive: true });
  try {
    const authorize = new URL(login.url);
    assert.equal(authorize.origin, "https://auth.openai.com");
    assert.equal(authorize.pathname, "/oauth/authorize");
    assert.equal(authorize.searchParams.get("scope"), "openid profile email offline_access");
    assert.equal(authorize.searchParams.get("code_challenge_method"), "S256");
    assert.equal(authorize.searchParams.get("originator"), "jecode");

    const redirect = new URL(authorize.searchParams.get("redirect_uri") as string);
    redirect.searchParams.set("state", authorize.searchParams.get("state") as string);
    redirect.searchParams.set("code", "authorization-code");
    const page = httpText(redirect, agent);
    const account = await within(login.complete(), 1_000, "browser login did not finish");

    const rendered = await page;
    assert.equal(rendered.status, 200);
    assert.equal(rendered.connection, "close");
    assert.match(rendered.body, /Signed in to Jecode/);
    assert.match(rendered.body, /data:image\/png;base64,/);
    assert.match(rendered.body, /history\.replaceState/);
    assert.equal(account.accessToken, accessToken("access-one"));
    assert.equal(account.refreshToken, "refresh-one");
    assert.equal(account.accountId, "account-1");
    assert.equal(account.email, "person@example.test");
    assert.equal(account.plan, "plus");
    const form = new URLSearchParams(exchangeBody);
    assert.equal(form.get("grant_type"), "authorization_code");
    assert.equal(form.get("code"), "authorization-code");
    const verifier = form.get("code_verifier") as string;
    assert.equal(
      authorize.searchParams.get("code_challenge"),
      createHash("sha256").update(verifier).digest("base64url"),
    );
  } finally {
    agent.destroy();
    await login.close();
  }
});

test("browser sign-in rejects a mismatched callback without cancelling the real login", async (context) => {
  const previousFetch = globalThis.fetch;
  let exchanged = false;
  globalThis.fetch = (async () => {
    exchanged = true;
    return json(tokens("unused", "unused"));
  }) as typeof fetch;
  context.after(() => { globalThis.fetch = previousFetch; });

  const login = await beginBrowserLogin();
  try {
    const authorize = new URL(login.url);
    const redirect = new URL(authorize.searchParams.get("redirect_uri") as string);
    redirect.searchParams.set("state", "wrong-state");
    redirect.searchParams.set("code", "authorization-code");
    const rejected = await httpText(redirect);

    assert.equal(rejected.status, 400);
    assert.equal(exchanged, false);

    redirect.searchParams.set("state", authorize.searchParams.get("state") as string);
    const accepted = httpText(redirect);
    const account = await within(login.complete(), 1_000, "browser login did not recover");

    assert.equal((await accepted).status, 200);
    assert.equal(account.accountId, "account-1");
    assert.equal(exchanged, true);
  } finally {
    await login.close();
  }
});

test("device sign-in exchanges only after the user code is authorized", async (context) => {
  const previousFetch = globalThis.fetch;
  const requests: { url: string; body: string }[] = [];
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input);
    requests.push({ url, body: String(init?.body ?? "") });
    if (url.endsWith("/api/accounts/deviceauth/usercode")) {
      return json({ device_auth_id: "device-1", user_code: "ABCD-EFGH", interval: 1 });
    }
    if (url.endsWith("/api/accounts/deviceauth/token")) {
      return json({ authorization_code: "device-auth-code", code_verifier: "device-verifier" });
    }
    return json(tokens("device-access", "device-refresh"));
  }) as typeof fetch;
  context.after(() => { globalThis.fetch = previousFetch; });

  const login = await beginDeviceLogin();
  assert.equal(login.url, "https://auth.openai.com/codex/device");
  assert.equal(login.code, "ABCD-EFGH");
  const account = await login.complete();

  assert.equal(account.accountId, "account-1");
  assert.equal(requests.length, 3);
  assert.match(requests[0]?.body ?? "", /"client_id"/);
  assert.match(requests[1]?.body ?? "", /"device_auth_id":"device-1"/);
  const exchange = new URLSearchParams(requests[2]?.body);
  assert.equal(exchange.get("redirect_uri"), "https://auth.openai.com/deviceauth/callback");
});

test("refresh keeps a rotated account identity and revoke uses the refresh token", async (context) => {
  const previousFetch = globalThis.fetch;
  const requests: { url: string; body: string }[] = [];
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    requests.push({ url: String(input), body: String(init?.body ?? "") });
    if (String(input).endsWith("/oauth/revoke")) return json({});
    return json(tokens("access-two", undefined));
  }) as typeof fetch;
  context.after(() => { globalThis.fetch = previousFetch; });
  const account: OpenAICodexAccount = {
    accessToken: "access-old",
    refreshToken: "refresh-old",
    expiresAt: Date.now() - 1,
    accountId: "account-1",
    email: "old@example.test",
  };

  const refreshed = await refreshOpenAITokens(account);
  await revokeOpenAITokens(refreshed);

  assert.equal(refreshed.accessToken, accessToken("access-two"));
  assert.equal(refreshed.refreshToken, "refresh-old");
  assert.equal(refreshed.email, "person@example.test");
  assert.match(requests[0]?.body ?? "", /"grant_type":"refresh_token"/);
  assert.match(requests[1]?.body ?? "", /"token":"refresh-old"/);
});

test("OAuth errors reject redirects and redact submitted secrets", async (context) => {
  const previousFetch = globalThis.fetch;
  let redirect = true;
  globalThis.fetch = (async () => redirect
    ? new Response(null, { status: 302, headers: { location: "https://evil.example/" } })
    : json({ message: "rejected secret-refresh-token" }, 400)) as typeof fetch;
  context.after(() => { globalThis.fetch = previousFetch; });
  const body = {
    contentType: "application/json" as const,
    value: { refresh_token: "secret-refresh-token" },
  };

  await assert.rejects(oauthRequest("https://auth.openai.com/oauth/token", body), /redirect rejected/);
  redirect = false;
  await assert.rejects(
    oauthRequest("https://auth.openai.com/oauth/token", body),
    (error: Error) => {
      assert.doesNotMatch(error.message, /secret-refresh-token/);
      assert.match(error.message, /credential redacted/);
      return true;
    },
  );
});

});

function tokens(access: string, refresh: string | undefined): Record<string, unknown> {
  return {
    access_token: accessToken(access),
    ...(refresh === undefined ? {} : { refresh_token: refresh }),
    expires_in: 3_600,
    id_token: jwt({ email: "person@example.test" }, "identity"),
  };
}

function accessToken(signature: string): string {
  return jwt({
    [CLAIMS]: { chatgpt_account_id: "account-1", chatgpt_plan_type: "plus" },
  }, signature);
}

function jwt(payload: Record<string, unknown>, signature: string): string {
  const header = Buffer.from(JSON.stringify({ alg: "none" })).toString("base64url");
  const body = Buffer.from(JSON.stringify(payload)).toString("base64url");
  return `${header}.${body}.${signature}`;
}

function json(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function httpText(
  url: URL,
  agent: Agent | false = false,
): Promise<{ status: number; body: string; connection?: string }> {
  return new Promise((resolve, reject) => {
    const request = get(url, { agent }, (response) => {
      let body = "";
      response.setEncoding("utf8");
      response.on("data", (chunk: string) => { body += chunk; });
      response.on("end", () => {
        clearTimeout(timer);
        resolve({ status: response.statusCode ?? 0, body, connection: response.headers.connection });
      });
    });
    const timer = setTimeout(() => request.destroy(new Error("callback response timed out")), 1_000);
    request.on("error", (error) => {
      clearTimeout(timer);
      reject(error);
    });
  });
}

async function within<T>(promise: Promise<T>, ms: number, message: string): Promise<T> {
  let timer: NodeJS.Timeout | undefined;
  const timeout = new Promise<never>((_resolve, reject) => {
    timer = setTimeout(() => reject(new Error(message)), ms);
  });
  try {
    return await Promise.race([promise, timeout]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}
