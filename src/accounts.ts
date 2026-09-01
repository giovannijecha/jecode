// OAuth accounts persisted under ~/.jecode, apart from API keys.

import { chmod, mkdir } from "node:fs/promises";
import { readFileSync } from "node:fs";
import * as path from "node:path";
import { atomicWrite } from "./atomic.ts";
import { withStoreLock } from "./store-lock.ts";
import { userDataLabel, userDataPath } from "./user-data.ts";

export type OpenAICodexAccount = {
  accessToken: string;
  refreshToken: string;
  expiresAt: number;
  accountId: string;
  email?: string;
  plan?: string;
};

type AccountStore = {
  version: 1;
  accounts: { "openai-codex"?: OpenAICodexAccount };
};

let cached: AccountStore | undefined;

export function openAICodexAccount(): OpenAICodexAccount | undefined {
  const account = store().accounts["openai-codex"];
  return account === undefined ? undefined : { ...account };
}

export function accountValues(): string[] {
  // Keep both snapshots. Another Jecode process may rotate the refresh token
  // after this process populated its account cache; shell-output redaction
  // must recognize the newly persisted values without forgetting the old ones.
  const values = new Set<string>();
  for (const source of [store(), readStore(accountsPath())]) {
    const account = source.accounts["openai-codex"];
    if (account === undefined) continue;
    values.add(account.accessToken);
    values.add(account.refreshToken);
  }
  return [...values];
}

export function accountsPath(): string {
  return userDataPath("accounts.json");
}

export function accountsLabel(): string {
  return userDataLabel("accounts.json");
}

export async function updateOpenAICodexAccount(
  change: (current: OpenAICodexAccount | undefined) => Promise<OpenAICodexAccount | undefined>,
  signal?: AbortSignal,
): Promise<OpenAICodexAccount | undefined> {
  const file = accountsPath();
  const directory = path.dirname(file);
  await mkdir(directory, { recursive: true, mode: 0o700 });
  if (process.platform !== "win32") await chmod(directory, 0o700);

  return withStoreLock(file, async () => {
    const current = readStore(file);
    const next = await change(current.accounts["openai-codex"]);
    const accounts = { ...current.accounts };
    if (next === undefined) delete accounts["openai-codex"];
    else accounts["openai-codex"] = { ...next };
    const updated: AccountStore = { version: 1, accounts };
    await atomicWrite(file, `${JSON.stringify(updated, null, 2)}\n`, { mode: 0o600 });
    cached = updated;
    return next === undefined ? undefined : { ...next };
  }, signal);
}

export function reloadAccounts(): void {
  cached = undefined;
}

function store(): AccountStore {
  if (cached === undefined) cached = readStore(accountsPath());
  return cached;
}

function readStore(file: string): AccountStore {
  try {
    const parsed = JSON.parse(readFileSync(file, "utf8")) as unknown;
    return normalize(parsed);
  } catch {
    return { version: 1, accounts: {} };
  }
}

function normalize(value: unknown): AccountStore {
  if (!record(value) || value["version"] !== 1 || !record(value["accounts"])) {
    return { version: 1, accounts: {} };
  }
  const account = normalizeOpenAI(value["accounts"]["openai-codex"]);
  return {
    version: 1,
    accounts: account === undefined ? {} : { "openai-codex": account },
  };
}

function normalizeOpenAI(value: unknown): OpenAICodexAccount | undefined {
  if (!record(value)) return undefined;
  const accessToken = nonempty(value["accessToken"]);
  const refreshToken = nonempty(value["refreshToken"]);
  const accountId = nonempty(value["accountId"]);
  const expiresAt = value["expiresAt"];
  if (
    accessToken === undefined ||
    refreshToken === undefined ||
    accountId === undefined ||
    typeof expiresAt !== "number" ||
    !Number.isSafeInteger(expiresAt) ||
    expiresAt <= 0
  ) return undefined;
  const email = nonempty(value["email"]);
  const plan = nonempty(value["plan"]);
  return {
    accessToken,
    refreshToken,
    expiresAt,
    accountId,
    ...(email === undefined ? {} : { email }),
    ...(plan === undefined ? {} : { plan }),
  };
}

function nonempty(value: unknown): string | undefined {
  return typeof value === "string" && value.trim() !== "" ? value : undefined;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
