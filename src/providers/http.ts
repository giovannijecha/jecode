// The entire HTTP layer: one bounded request, then either a JSON body or an
// event stream. Only idempotent reads retry. Once a POST starts or response
// bytes flow, a failure is surfaced rather than silently replayed.

import { leadingText } from "../text-boundary.ts";
import { readSseJson } from "./sse.ts";
import { sseStreamCharacterLimit } from "./stream-limits.ts";

const RETRYABLE = new Set([408, 409, 429, 500, 502, 503, 504]);
const MAX_JSON_CHARS = 5_000_000;
const MAX_ERROR_CHARS = 2_000;
const HANDSHAKE_TIMEOUT_MS = 60_000;
const BODY_IDLE_TIMEOUT_MS = 120_000;
const GET_RETRIES = 3;

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
  const { text, truncated } = await boundedText(url, res, MAX_JSON_CHARS);
  if (truncated) {
    throw httpError(`${url} returned JSON over ${MAX_JSON_CHARS} characters`, res.status, text);
  }
  try {
    return JSON.parse(text) as unknown;
  } catch {
    throw httpError(`${url} returned non-JSON`, res.status, leadingText(text, 500));
  }
}

export async function postSse(
  url: string,
  headers: Record<string, string>,
  body: unknown,
  maxOutputTokens: number,
  signal?: AbortSignal,
  onStatus?: HttpStatus,
): Promise<AsyncGenerator<unknown>> {
  const maximumChars = sseStreamCharacterLimit(maxOutputTokens);
  const res = await request(url, { accept: "text/event-stream", ...headers }, body, signal, onStatus);
  if (res.body === null) throw httpError(`${url} returned no body`, res.status);
  return readSseJson(res.body, maximumChars, {
    milliseconds: BODY_IDLE_TIMEOUT_MS,
    error: () => httpError(
      `${url} SSE stream was idle for ${BODY_IDLE_TIMEOUT_MS}ms without an event`,
      res.status,
    ),
  });
}

async function request(
  url: string,
  headers: Record<string, string>,
  body: unknown,
  signal: AbortSignal | undefined,
  onStatus: HttpStatus | undefined,
): Promise<Response> {
  const maxRetries = body === undefined ? GET_RETRIES : 0;
  let lastError: HttpError | undefined;
  let waitMs = 0;

  for (let attempt = 0; attempt <= maxRetries; attempt++) {
    if (waitMs > 0) await sleep(waitMs, signal);

    let res: Response;
    const handshake = handshakeSignal(url, signal);
    try {
      // No body, no method: a request with nothing to send is a read, and
      // saying so is what keeps the retry and error handling in one place.
      res = await fetch(url, {
        method: body === undefined ? "GET" : "POST",
        headers: body === undefined ? headers : { "content-type": "application/json", ...headers },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
        redirect: "manual",
        signal: handshake.signal,
      });
    } catch (cause) {
      const timeout = handshake.timeout();
      if (timeout !== undefined) throw timeout;
      if (signal?.aborted === true) throw cause;
      const detail = cause instanceof Error ? cause.message : String(cause);
      lastError = httpError(`network error calling ${url}: ${detail}`);
      waitMs = backoff(attempt);
      if (attempt < maxRetries) onStatus?.(`Network error · retrying in ${waitLabel(waitMs)}`);
      continue;
    } finally {
      handshake.clear();
    }

    if (res.status >= 300 && res.status < 400) {
      await res.body?.cancel().catch(() => undefined);
      throw httpError(`${url} -> ${res.status} redirect rejected`, res.status);
    }
    if (res.ok) return res;

    const { text } = await boundedText(url, res, MAX_ERROR_CHARS);
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

async function boundedText(
  url: string,
  res: Response,
  max: number,
): Promise<{ text: string; truncated: boolean }> {
  if (res.body === null) return { text: "", truncated: false };
  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let text = "";

  try {
    while (true) {
      const { done, value } = await timedRead(url, reader);
      if (done) {
        text += decoder.decode();
        return text.length > max
          ? { text: leadingText(text, max), truncated: true }
          : { text, truncated: false };
      }
      text += decoder.decode(value, { stream: true });
      if (text.length > max) {
        await reader.cancel().catch(() => undefined);
        return { text: leadingText(text, max), truncated: true };
      }
    }
  } finally {
    reader.releaseLock();
  }
}

function handshakeSignal(
  url: string,
  signal: AbortSignal | undefined,
): { signal: AbortSignal; timeout(): HttpError | undefined; clear(): void } {
  const controller = new AbortController();
  let timeout: HttpError | undefined;
  const timer = setTimeout(() => {
    timeout = httpError(
      `${url} timed out waiting for response headers after ${HANDSHAKE_TIMEOUT_MS}ms`,
    );
    controller.abort(timeout);
  }, HANDSHAKE_TIMEOUT_MS);

  return {
    signal: signal === undefined ? controller.signal : AbortSignal.any([signal, controller.signal]),
    timeout: () => timeout,
    clear: () => clearTimeout(timer),
  };
}

async function timedRead(
  url: string,
  reader: ReadableStreamDefaultReader<Uint8Array>,
): Promise<ReadableStreamReadResult<Uint8Array>> {
  const timeout = httpError(
    `${url} response body was idle for ${BODY_IDLE_TIMEOUT_MS}ms`,
  );
  let timer: NodeJS.Timeout | undefined;
  const expired = new Promise<never>((_resolve, reject) => {
    timer = setTimeout(() => reject(timeout), BODY_IDLE_TIMEOUT_MS);
  });

  try {
    return await Promise.race([reader.read(), expired]);
  } catch (error) {
    if (error === timeout) await reader.cancel(timeout).catch(() => undefined);
    throw error;
  } finally {
    if (timer !== undefined) clearTimeout(timer);
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
