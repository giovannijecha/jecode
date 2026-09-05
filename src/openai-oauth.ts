// OpenAI's ChatGPT OAuth protocol, implemented with Node primitives only.
//
// This module owns the authority handshake. It does not know about the TUI or
// persist anything; callers decide when an explicitly completed login becomes
// an account on disk.

import { createHash, randomBytes } from "node:crypto";
import type { OpenAICodexAccount } from "./accounts.ts";
import { oauthRequest } from "./oauth-http.ts";
import { providerLabel } from "./provider-label.ts";
import { OPENAI_CALLBACK_PATH, openAICallback } from "./openai-oauth-callback.ts";
import {
  openAIAccountFromTokens,
  openAITokenReply,
  type OpenAITokenReply,
} from "./openai-oauth-tokens.ts";

const ACCOUNT_LABEL = providerLabel("openai-codex");
const CLIENT_ID = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTHORITY = "https://auth.openai.com";
const AUTHORIZE = `${AUTHORITY}/oauth/authorize`;
const TOKEN = `${AUTHORITY}/oauth/token`;
const REVOKE = `${AUTHORITY}/oauth/revoke`;
const DEVICE_CODE = `${AUTHORITY}/api/accounts/deviceauth/usercode`;
const DEVICE_POLL = `${AUTHORITY}/api/accounts/deviceauth/token`;
const DEVICE_VERIFY = `${AUTHORITY}/codex/device`;
const DEVICE_REDIRECT = `${AUTHORITY}/deviceauth/callback`;
const LOGIN_LIMIT_MS = 15 * 60_000;
/** RFC 8628 increases the polling interval by five seconds after `slow_down`. */
const SLOW_DOWN_INCREMENT_MS = 5_000;

export type OpenAILogin = {
  url: string;
  code?: string;
  complete(signal?: AbortSignal): Promise<OpenAICodexAccount>;
  close(): Promise<void>;
};

type PendingCode = {
  authorizationCode: string;
  verifier: string;
  redirectUri: string;
};

export async function beginBrowserLogin(): Promise<OpenAILogin> {
  const verifier = randomBytes(64).toString("base64url");
  const challenge = createHash("sha256").update(verifier).digest("base64url");
  const state = randomBytes(32).toString("base64url");
  const callback = await openAICallback(state);
  const redirectUri = `http://localhost:${callback.port}${OPENAI_CALLBACK_PATH}`;
  const authorize = new URL(AUTHORIZE);
  authorize.search = new URLSearchParams({
    response_type: "code",
    client_id: CLIENT_ID,
    redirect_uri: redirectUri,
    scope: "openid profile email offline_access",
    code_challenge: challenge,
    code_challenge_method: "S256",
    state,
    id_token_add_organizations: "true",
    codex_cli_simplified_flow: "true",
    originator: "jecode",
  }).toString();

  return {
    url: authorize.href,
    async complete(signal?: AbortSignal) {
      try {
        const code = await abortable(callback.code, signal);
        const account = openAIAccountFromTokens(await exchange({
          authorizationCode: code,
          verifier,
          redirectUri,
        }, signal));
        await callback.respond(true);
        return account;
      } catch (error) {
        await callback.respond(false);
        throw error;
      } finally {
        await callback.close();
      }
    },
    close: callback.close,
  };
}

export async function beginDeviceLogin(signal?: AbortSignal): Promise<OpenAILogin> {
  const start = await oauthRequest(
    DEVICE_CODE,
    { contentType: "application/json", value: { client_id: CLIENT_ID } },
    signal,
  );
  const value = record(start.value) ? start.value : {};
  const deviceAuthId = required(value["device_auth_id"], "device authorization id");
  const code = required(value["user_code"] ?? value["usercode"], "device code");
  const interval = intervalSeconds(value["interval"]);

  return {
    url: DEVICE_VERIFY,
    code,
    async complete(waitSignal?: AbortSignal) {
      const combined = combine(signal, waitSignal);
      const pending = await pollDevice(deviceAuthId, code, interval, combined);
      return openAIAccountFromTokens(await exchange(pending, combined));
    },
    close: async () => {},
  };
}

export async function refreshOpenAITokens(
  account: OpenAICodexAccount,
  signal?: AbortSignal,
): Promise<OpenAICodexAccount> {
  const response = await oauthRequest(
    TOKEN,
    {
      contentType: "application/json",
      value: {
        client_id: CLIENT_ID,
        grant_type: "refresh_token",
        refresh_token: account.refreshToken,
      },
    },
    signal,
  );
  const token = openAITokenReply(response.value, account.refreshToken);
  const refreshed = openAIAccountFromTokens(token);
  if (refreshed.accountId !== account.accountId) {
    throw new Error(`${ACCOUNT_LABEL} refresh returned a different account · sign in again`);
  }
  return {
    ...refreshed,
    ...(refreshed.email === undefined && account.email !== undefined ? { email: account.email } : {}),
    ...(refreshed.plan === undefined && account.plan !== undefined ? { plan: account.plan } : {}),
  };
}

export async function revokeOpenAITokens(
  account: OpenAICodexAccount,
  signal?: AbortSignal,
): Promise<void> {
  await oauthRequest(
    REVOKE,
    {
      contentType: "application/json",
      value: {
        token: account.refreshToken,
        token_type_hint: "refresh_token",
        client_id: CLIENT_ID,
      },
    },
    signal,
  );
}

async function exchange(code: PendingCode, signal?: AbortSignal): Promise<OpenAITokenReply> {
  const response = await oauthRequest(
    TOKEN,
    {
      contentType: "application/x-www-form-urlencoded",
      value: new URLSearchParams({
        grant_type: "authorization_code",
        client_id: CLIENT_ID,
        code: code.authorizationCode,
        code_verifier: code.verifier,
        redirect_uri: code.redirectUri,
      }),
    },
    signal,
  );
  return openAITokenReply(response.value);
}

async function pollDevice(
  deviceAuthId: string,
  userCode: string,
  interval: number,
  signal?: AbortSignal,
): Promise<PendingCode> {
  const deadline = new AbortController();
  const timer = setTimeout(() => {
    deadline.abort(new Error(`${ACCOUNT_LABEL} device sign-in timed out after 15 minutes`));
  }, LOGIN_LIMIT_MS);
  const combined = signal === undefined
    ? deadline.signal
    : AbortSignal.any([signal, deadline.signal]);
  let intervalMs = interval * 1_000;
  try {
    while (true) {
      const response = await oauthRequest(
        DEVICE_POLL,
        {
          contentType: "application/json",
          value: { device_auth_id: deviceAuthId, user_code: userCode },
        },
        combined,
        [200, 400, 403, 404, 429],
      );
      if (combined.aborted) throw abortReason(combined);
      if (response.status === 200) {
        const value = record(response.value) ? response.value : {};
        return {
          authorizationCode: required(value["authorization_code"], "authorization code"),
          verifier: required(value["code_verifier"], "code verifier"),
          redirectUri: DEVICE_REDIRECT,
        };
      }

      const errorCode = deviceErrorCode(response.value);
      if (errorCode === "access_denied") {
        throw new Error(`${ACCOUNT_LABEL} device sign-in was denied`);
      }
      if (errorCode === "expired_token") {
        throw new Error(`${ACCOUNT_LABEL} device sign-in code expired`);
      }

      const pending = errorCode === "authorization_pending" ||
        errorCode === "deviceauth_authorization_pending" ||
        ((response.status === 403 || response.status === 404) && errorCode === undefined);
      const slowDown = errorCode === "slow_down" ||
        (response.status === 429 && errorCode === undefined);
      if (!pending && !slowDown) {
        throw new Error(`${ACCOUNT_LABEL} device sign-in failed (${response.status})`);
      }
      if (slowDown) intervalMs += SLOW_DOWN_INCREMENT_MS;
      await sleep(intervalMs, combined);
    }
  } finally {
    clearTimeout(timer);
  }
}

function deviceErrorCode(value: unknown): string | undefined {
  if (!record(value)) return undefined;
  const error = value["error"];
  const nested = typeof error === "string"
    ? error
    : record(error)
      ? optional(error["code"])
      : undefined;
  const code = nested ?? optional(value["code"]);
  return code?.trim().toLowerCase();
}

function intervalSeconds(value: unknown): number {
  const parsed = typeof value === "string" ? Number(value.trim()) : value;
  return typeof parsed === "number" && Number.isFinite(parsed)
    ? Math.max(1, Math.min(30, Math.floor(parsed)))
    : 5;
}

function required(value: unknown, label: string): string {
  const found = optional(value);
  if (found === undefined) throw new Error(`${ACCOUNT_LABEL} sign-in returned no ${label}`);
  return found;
}

function optional(value: unknown): string | undefined {
  return typeof value === "string" && value !== "" ? value : undefined;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function combine(left?: AbortSignal, right?: AbortSignal): AbortSignal | undefined {
  if (left === undefined) return right;
  if (right === undefined) return left;
  return AbortSignal.any([left, right]);
}

function abortable<T>(promise: Promise<T>, signal?: AbortSignal): Promise<T> {
  if (signal === undefined) return promise;
  if (signal.aborted) return Promise.reject(abortReason(signal));
  return new Promise((resolve, reject) => {
    const onAbort = () => reject(abortReason(signal));
    signal.addEventListener("abort", onAbort, { once: true });
    promise.then(
      (value) => {
        signal.removeEventListener("abort", onAbort);
        resolve(value);
      },
      (error) => {
        signal.removeEventListener("abort", onAbort);
        reject(error as Error);
      },
    );
  });
}

function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted === true) {
      reject(abortReason(signal));
      return;
    }
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, ms);
    const onAbort = () => {
      clearTimeout(timer);
      reject(signal === undefined ? new Error("cancelled") : abortReason(signal));
    };
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

function abortReason(signal: AbortSignal): Error {
  return signal.reason instanceof Error ? signal.reason : new Error("cancelled");
}
