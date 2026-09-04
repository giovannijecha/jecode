// OAuth accounts persisted under ~/.jecode, apart from API keys.

import * as path from "node:path";
import { atomicWrite } from "./atomic.ts";
import {
  assertDirectoryAnchor,
  captureDirectDirectorySync,
  preparePrivateDirectory,
} from "./directory-anchor.ts";
import type { DirectoryAnchor } from "./directory-anchor.ts";
import { withStoreLock } from "./store-lock.ts";
import { userDataLabel, userDataPath } from "./user-data.ts";
import {
  assertStoreText,
  readBoundedJsonForMutationSync,
  readBoundedJsonSync,
  USER_STORE_LIMITS,
} from "./user-store.ts";

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
  const anchor = await preparePrivateDirectory(directory, "account store directory");
  const anchoredFile = path.join(anchor.path, path.basename(file));

  return withStoreLock(anchoredFile, async () => {
    const current = readStoreForMutation(anchoredFile, anchor);
    const next = await change(current.accounts["openai-codex"]);
    const normalized = next === undefined ? undefined : normalizeOpenAI(next);
    if (next !== undefined && normalized === undefined) throw new Error("invalid OpenAI account");
    const accounts = { ...current.accounts };
    if (next === undefined) delete accounts["openai-codex"];
    else accounts["openai-codex"] = normalized as OpenAICodexAccount;
    const updated: AccountStore = { version: 1, accounts };
    const text = `${JSON.stringify(updated, null, 2)}\n`;
    assertStoreText(text, USER_STORE_LIMITS.accountsBytes);
    await atomicWrite(anchoredFile, text, {
      mode: 0o600,
      validate: async () => assertDirectoryAnchor(anchor),
    });
    cached = updated;
    return normalized === undefined ? undefined : { ...normalized };
  }, signal, async () => assertDirectoryAnchor(anchor));
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
    const directory = captureDirectDirectorySync(path.dirname(file), "account store directory");
    const anchoredFile = path.join(directory.path, path.basename(file));
    return normalize(readBoundedJsonSync(
      anchoredFile,
      USER_STORE_LIMITS.accountsBytes,
      directory,
    ));
  } catch {
    return { version: 1, accounts: {} };
  }
}

function readStoreForMutation(file: string, directory: DirectoryAnchor): AccountStore {
  const value = readBoundedJsonForMutationSync(
    file,
    USER_STORE_LIMITS.accountsBytes,
    "account store",
    directory,
  );
  if (value === undefined) return { version: 1, accounts: {} };
  if (
    !record(value) || value["version"] !== 1 || !record(value["accounts"]) ||
    hasUnknownKeys(value, ["version", "accounts"])
  ) {
    throw new Error("account store has an unsupported structure");
  }
  const accounts = value["accounts"];
  if (Object.keys(accounts).some((name) => name !== "openai-codex")) {
    throw new Error("account store has an unsupported structure");
  }
  const raw = accounts["openai-codex"];
  if (raw === undefined) return { version: 1, accounts: {} };
  if (
    !record(raw) || hasUnknownKeys(raw, [
      "accessToken",
      "refreshToken",
      "expiresAt",
      "accountId",
      "email",
      "plan",
    ])
  ) throw new Error("account store has an unsupported structure");
  const account = normalizeOpenAI(raw);
  if (account === undefined) throw new Error("account store has an invalid OpenAI account");
  return { version: 1, accounts: { "openai-codex": account } };
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
  const accessToken = nonempty(value["accessToken"], USER_STORE_LIMITS.accountToken);
  const refreshToken = nonempty(value["refreshToken"], USER_STORE_LIMITS.accountToken);
  const accountId = nonempty(value["accountId"], USER_STORE_LIMITS.accountLabel);
  const expiresAt = value["expiresAt"];
  if (
    accessToken === undefined ||
    refreshToken === undefined ||
    accountId === undefined ||
    typeof expiresAt !== "number" ||
    !Number.isSafeInteger(expiresAt) ||
    expiresAt <= 0
  ) return undefined;
  const email = nonempty(value["email"], USER_STORE_LIMITS.accountLabel);
  const plan = nonempty(value["plan"], USER_STORE_LIMITS.accountLabel);
  return {
    accessToken,
    refreshToken,
    expiresAt,
    accountId,
    ...(email === undefined ? {} : { email }),
    ...(plan === undefined ? {} : { plan }),
  };
}

function nonempty(value: unknown, max: number): string | undefined {
  return typeof value === "string" && value.length <= max && value.trim() !== "" ? value : undefined;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasUnknownKeys(value: Record<string, unknown>, known: readonly string[]): boolean {
  const allowed = new Set(known);
  return Object.keys(value).some((key) => !allowed.has(key));
}
