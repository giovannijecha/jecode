// The entire HTTP layer: one retrying request, then either a JSON body or an
// event stream. Retries wrap only the handshake — once bytes are flowing, a
// failure is the caller's to surface, never something to silently replay.

import { readSseJson } from "./sse.ts";

const RETRYABLE = new Set([408, 409, 429, 500, 502, 503, 504]);
const MAX_JSON_CHARS = 5_000_000;
const MAX_ERROR_CHARS = 2_000;

export type HttpError = Error & { status?: number; body?: string };
export type HttpStatus = (text: string) => void;

function httpError(message: string, status?: number, body?: string): HttpError {
  const error = new Error(message) as HttpError;
  error.status = status;
  error.body = body;
  return error;
}

export async function postJson(
  url: string,
  headers: Record<string, string>,
  body: unknown,
  signal?: AbortSignal,
  onStatus?: HttpStatus,
): Promise<unknown> {
  return asJson(url, await request(url, headers, body, signal, onStatus));
}

/** A plain read. The only thing jecode asks for without sending anything. */
export async function getJson(
  url: string,
  headers: Record<string, string>,
  signal?: AbortSignal,
  onStatus?: HttpStatus,
): Promise<unknown> {
  return asJson(url, await request(url, headers, undefined, signal, onStatus));
}

async function asJson(url: string, res: Response): Promise<unknown> {
  const { text, truncated } = await boundedText(res, MAX_JSON_CHARS);
  if (truncated) {
    throw httpError(`${url} returned JSON over ${MAX_JSON_CHARS} characters`, res.status, text);
  }
  try {
    return JSON.parse(text) as unknown;
  } catch {
    throw httpError(`${url} returned non-JSON`, res.status, text.slice(0, 500));
  }
}

export async function postSse(
  url: string,
  headers: Record<string, string>,
  body: unknown,
  signal?: AbortSignal,
  onStatus?: HttpStatus,
): Promise<AsyncGenerator<unknown>> {
  const res = await request(url, { accept: "text/event-stream", ...headers }, body, signal, onStatus);
  if (res.body === null) throw httpError(`${url} returned no body`, res.status);
  return readSseJson(res.body);
}

async function request(
  url: string,
  headers: Record<string, string>,
  body: unknown,
  signal: AbortSignal | undefined,
  onStatus: HttpStatus | undefined,
  maxRetries = 3,
): Promise<Response> {
  let lastError: HttpError | undefined;
  let waitMs = 0;

  for (let attempt = 0; attempt <= maxRetries; attempt++) {
    if (waitMs > 0) await sleep(waitMs, signal);

    let res: Response;
    try {
      // No body, no method: a request with nothing to send is a read, and
      // saying so is what keeps the retry and error handling in one place.
      res = await fetch(url, {
        method: body === undefined ? "GET" : "POST",
        headers: body === undefined ? headers : { "content-type": "application/json", ...headers },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
        signal,
      });
    } catch (cause) {
      if (signal?.aborted === true) throw cause;
      lastError = httpError(`network error calling ${url}: ${(cause as Error).message}`);
      waitMs = backoff(attempt);
      if (attempt < maxRetries) onStatus?.(`Network error · retrying in ${waitLabel(waitMs)}`);
      continue;
    }

    if (res.ok) return res;

    const { text } = await boundedText(res, MAX_ERROR_CHARS);
    lastError = httpError(
      `${url} -> ${res.status} ${res.statusText}`,
      res.status,
      text,
    );
    if (!RETRYABLE.has(res.status)) throw lastError;
    waitMs = retryAfter(res) ?? backoff(attempt);
    if (attempt < maxRetries) {
      const reason = res.status === 429 ? "Rate limited" : `HTTP ${res.status}`;
      onStatus?.(`${reason} · retrying in ${waitLabel(waitMs)}`);
    }
  }

  throw lastError ?? httpError(`${url} failed`);
}

async function boundedText(res: Response, max: number): Promise<{ text: string; truncated: boolean }> {
  if (res.body === null) return { text: "", truncated: false };
  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let text = "";

  while (true) {
    const { done, value } = await reader.read();
    if (done) {
      text += decoder.decode();
      return text.length > max
        ? { text: text.slice(0, max), truncated: true }
        : { text, truncated: false };
    }
    text += decoder.decode(value, { stream: true });
    if (text.length > max) {
      await reader.cancel().catch(() => undefined);
      return { text: text.slice(0, max), truncated: true };
    }
  }
}

function waitLabel(ms: number): string {
  return ms < 1_000 ? `${ms}ms` : `${Math.ceil(ms / 1_000)}s`;
}

function backoff(attempt: number): number {
  return Math.min(30_000, 1_000 * 2 ** attempt);
}

function retryAfter(res: Response): number | undefined {
  const header = res.headers.get("retry-after");
  if (header === null) return undefined;

  const seconds = Number(header);
  if (Number.isFinite(seconds) && seconds >= 0) return Math.min(60_000, seconds * 1_000);

  const at = Date.parse(header);
  return Number.isNaN(at) ? undefined : Math.max(0, Math.min(60_000, at - Date.now()));
}

function sleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted === true) {
      reject(signal.reason as Error);
      return;
    }
    const onAbort = () => {
      clearTimeout(timer);
      reject(signal?.reason as Error);
    };
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, ms);
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}
