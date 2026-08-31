// Validate OAuth token responses and extract the ChatGPT account claims.

import type { OpenAICodexAccount } from "./accounts.ts";

const CLAIMS = "https://api.openai.com/auth";

export type OpenAITokenReply = {
  accessToken: string;
  refreshToken: string;
  expiresAt: number;
  idToken?: string;
};

export function openAITokenReply(
  value: unknown,
  previousRefresh?: string,
): OpenAITokenReply {
  if (!record(value)) throw new Error("OpenAI sign-in returned an invalid token response");
  const accessToken = required(value["access_token"], "access token");
  const refreshToken = optional(value["refresh_token"]) ?? previousRefresh;
  const expiresIn = value["expires_in"];
  if (refreshToken === undefined) throw new Error("OpenAI sign-in did not return a refresh token");
  if (typeof expiresIn !== "number" || !Number.isFinite(expiresIn) || expiresIn <= 0) {
    throw new Error("OpenAI sign-in did not return a valid token lifetime");
  }
  const idToken = optional(value["id_token"]);
  return {
    accessToken,
    refreshToken,
    expiresAt: Date.now() + Math.floor(expiresIn * 1_000),
    ...(idToken === undefined ? {} : { idToken }),
  };
}

export function openAIAccountFromTokens(token: OpenAITokenReply): OpenAICodexAccount {
  const access = jwt(token.accessToken);
  const identity = token.idToken === undefined ? undefined : jwt(token.idToken);
  const auth = record(access[CLAIMS]) ? access[CLAIMS] : {};
  const accountId = optional(auth["chatgpt_account_id"]) ?? optional(identity?.["chatgpt_account_id"]);
  if (accountId === undefined) throw new Error("OpenAI sign-in did not identify a ChatGPT account");
  const email = optional(identity?.["email"]);
  const plan = optional(auth["chatgpt_plan_type"]) ?? optional(identity?.["chatgpt_plan_type"]);
  return {
    accessToken: token.accessToken,
    refreshToken: token.refreshToken,
    expiresAt: token.expiresAt,
    accountId,
    ...(email === undefined ? {} : { email }),
    ...(plan === undefined ? {} : { plan }),
  };
}

function jwt(token: string): Record<string, unknown> {
  const part = token.split(".")[1];
  if (part === undefined) throw new Error("OpenAI sign-in returned an unreadable token");
  try {
    const value = JSON.parse(Buffer.from(part, "base64url").toString("utf8")) as unknown;
    if (!record(value)) throw new Error("invalid payload");
    return value;
  } catch {
    throw new Error("OpenAI sign-in returned an unreadable token");
  }
}

function required(value: unknown, label: string): string {
  const found = optional(value);
  if (found === undefined) throw new Error(`OpenAI sign-in returned no ${label}`);
  return found;
}

function optional(value: unknown): string | undefined {
  return typeof value === "string" && value !== "" ? value : undefined;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
