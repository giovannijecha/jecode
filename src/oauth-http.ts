// A narrow HTTP boundary for OAuth authority requests.
//
// These requests never redirect, never retry, and never retain an unbounded
// response. Authorization codes and refresh tokens must not leak into errors.

const AUTH_ORIGIN = "https://auth.openai.com";
const TIMEOUT_MS = 15_000;
const MAX_BODY_CHARS = 64_000;

export type OAuthBody =
  | { contentType: "application/json"; value: unknown }
  | { contentType: "application/x-www-form-urlencoded"; value: URLSearchParams };

export type OAuthResponse = { status: number; value: unknown };

export async function oauthRequest(
  url: string,
  body: OAuthBody,
  signal?: AbortSignal,
  accepted: readonly number[] = [200],
): Promise<OAuthResponse> {
  const target = new URL(url);
  if (target.origin !== AUTH_ORIGIN || target.username !== "" || target.password !== "") {
    throw new Error("OAuth request target is not allowed");
  }

  const timeout = new AbortController();
  const timer = setTimeout(() => timeout.abort(new Error("OpenAI sign-in timed out")), TIMEOUT_MS);
  const combined = signal === undefined
    ? timeout.signal
    : AbortSignal.any([signal, timeout.signal]);
  const secrets = bodySecrets(body);

  let response: Response;
  let text: string;
  try {
    response = await fetch(target, {
      method: "POST",
      headers: { "content-type": body.contentType, accept: "application/json" },
      body: body.contentType === "application/json"
        ? JSON.stringify(body.value)
        : body.value.toString(),
      cache: "no-store",
      redirect: "manual",
      signal: combined,
    });
    if (response.status >= 300 && response.status < 400) {
      await response.body?.cancel().catch(() => undefined);
      throw new Error(`OpenAI sign-in redirect rejected (${response.status})`);
    }
    text = await boundedText(response);
  } catch (error) {
    if (timeout.signal.aborted) throw timeout.signal.reason;
    if (signal?.aborted === true) throw abortReason(signal);
    if (error instanceof Error && error.message.startsWith("OpenAI sign-in")) throw error;
    const detail = error instanceof Error ? error.message : String(error);
    throw new Error(`OpenAI sign-in network error: ${detail}`);
  } finally {
    clearTimeout(timer);
  }
  const value = parse(text);
  if (!accepted.includes(response.status)) {
    throw new Error(`OpenAI sign-in failed (${response.status})${errorDetail(value, secrets)}`);
  }
  return { status: response.status, value };
}

async function boundedText(response: Response): Promise<string> {
  if (response.body === null) return "";
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let text = "";
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) return text + decoder.decode();
      text += decoder.decode(value, { stream: true });
      if (text.length > MAX_BODY_CHARS) {
        await reader.cancel().catch(() => undefined);
        throw new Error("OpenAI sign-in returned too much data");
      }
    }
  } finally {
    reader.releaseLock();
  }
}

function parse(text: string): unknown {
  if (text.trim() === "") return {};
  try {
    return JSON.parse(text) as unknown;
  } catch {
    throw new Error("OpenAI sign-in returned an invalid response");
  }
}

function errorDetail(value: unknown, secrets: readonly string[]): string {
  if (!record(value)) return "";
  const detail = [value["error_description"], value["message"], value["error"]]
    .find((entry) => typeof entry === "string");
  if (typeof detail !== "string" || detail.trim() === "") return "";
  let safe = detail.replace(/[\r\n]+/g, " ");
  for (const secret of secrets) safe = safe.replaceAll(secret, "[credential redacted]");
  return ` · ${safe.slice(0, 300)}`;
}

function bodySecrets(body: OAuthBody): string[] {
  const values = body.contentType === "application/x-www-form-urlencoded"
    ? [...body.value.values()]
    : record(body.value)
      ? Object.values(body.value).filter((value): value is string => typeof value === "string")
      : [];
  return values.filter((value) => value.length >= 8);
}

function abortReason(signal: AbortSignal): Error {
  return signal.reason instanceof Error ? signal.reason : new Error("cancelled");
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
