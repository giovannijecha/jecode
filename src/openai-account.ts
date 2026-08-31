// The live ChatGPT account: persistence, proactive refresh, and logout.

import type { OpenAICodexAccount } from "./accounts.ts";
import { openAICodexAccount, updateOpenAICodexAccount } from "./accounts.ts";
import { refreshOpenAITokens, revokeOpenAITokens } from "./openai-oauth.ts";

const REFRESH_EARLY_MS = 5 * 60_000;
let refreshing: Promise<OpenAICodexAccount> | undefined;

export type OpenAIAuthorization = { accessToken: string; accountId: string };

export function openAIAccountHint(): string {
  const account = openAICodexAccount();
  if (account === undefined) return "not connected";
  const identity = account.email ?? "connected";
  return account.plan === undefined ? identity : `${identity} · ${account.plan}`;
}

export async function saveOpenAIAccount(
  account: OpenAICodexAccount,
  signal?: AbortSignal,
): Promise<void> {
  await updateOpenAICodexAccount(async () => account, signal);
}

export async function removeOpenAIAccount(signal?: AbortSignal): Promise<{
  removed: boolean;
  revokeFailed: boolean;
}> {
  let account: OpenAICodexAccount | undefined;
  await updateOpenAICodexAccount(async (current) => {
    account = current;
    return undefined;
  }, signal);
  if (account === undefined) return { removed: false, revokeFailed: false };

  let revokeFailed = false;
  try {
    await revokeOpenAITokens(account, signal);
  } catch {
    revokeFailed = true;
  }
  return { removed: true, revokeFailed };
}

export async function openAIAuthorization(
  forceToken?: string,
  signal?: AbortSignal,
  onStatus?: (status: string) => void,
): Promise<OpenAIAuthorization> {
  const account = openAICodexAccount();
  if (account === undefined) throw new Error("ChatGPT account is not connected");

  const mustRefresh = forceToken !== undefined || expiresSoon(account);
  const ready = mustRefresh
    ? await refreshAccount(forceToken, signal, onStatus)
    : account;
  return { accessToken: ready.accessToken, accountId: ready.accountId };
}

async function refreshAccount(
  forceToken: string | undefined,
  signal: AbortSignal | undefined,
  onStatus: ((status: string) => void) | undefined,
): Promise<OpenAICodexAccount> {
  if (refreshing !== undefined) return abortable(refreshing, signal);
  onStatus?.("Refreshing ChatGPT sign-in");
  const task = updateOpenAICodexAccount(async (current) => {
    if (current === undefined) throw new Error("ChatGPT account is not connected");
    if (forceToken !== undefined && current.accessToken !== forceToken) return current;
    if (forceToken === undefined && !expiresSoon(current)) return current;
    return refreshOpenAITokens(current, signal);
  }, signal).then((account) => {
    if (account === undefined) throw new Error("ChatGPT account is not connected");
    return account;
  });
  refreshing = task;
  try {
    return await abortable(task, signal);
  } finally {
    if (refreshing === task) refreshing = undefined;
  }
}

function expiresSoon(account: OpenAICodexAccount): boolean {
  return account.expiresAt - Date.now() <= REFRESH_EARLY_MS;
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

function abortReason(signal: AbortSignal): Error {
  return signal.reason instanceof Error ? signal.reason : new Error("cancelled");
}
