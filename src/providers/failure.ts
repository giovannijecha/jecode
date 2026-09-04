// Provider failures normalized at adapter boundaries. The controller and TUI
// never need to understand a vendor response shape in order to make a safe
// retry decision or show an actionable error.

import { leadingText } from "../text-boundary.ts";
import type { HttpError } from "./http.ts";

const MAX_WIRE_ERROR_CHARS = 2_000;

export type ProviderFailureKind =
  | "authentication"
  | "billing"
  | "quota"
  | "rate-limit"
  | "overload"
  | "context"
  | "network"
  | "unknown";

export type ProviderFailureDetails = Readonly<{
  kind: ProviderFailureKind;
  status?: number;
  code?: string;
  retryAfterMs?: number;
  requestId?: string;
}>;

type WireError = Error & {
  code?: string;
  type?: string;
};

/** One stable error contract for every provider adapter. */
export class ProviderRequestError extends Error {
  readonly providerId: string;
  readonly kind: ProviderFailureKind;
  readonly status: number | undefined;
  readonly code: string | undefined;
  readonly retryAfterMs: number | undefined;
  readonly requestId: string | undefined;
  readonly body: string | undefined;

  constructor(providerId: string, source: Error, details: ProviderFailureDetails) {
    super(source.message, { cause: source });
    this.name = "ProviderRequestError";
    this.providerId = providerId;
    this.kind = details.kind;
    this.status = details.status;
    this.code = details.code;
    this.retryAfterMs = details.retryAfterMs;
    this.requestId = details.requestId;
    this.body = (source as HttpError).body;
  }
}

/** Preserve structured fields carried by an error event inside a live stream. */
export function providerWireError(
  prefix: string,
  message: string | undefined,
  metadata: { code?: unknown; type?: unknown } = {},
): Error {
  const detail = message === undefined ? "unspecified" : leadingText(message, MAX_WIRE_ERROR_CHARS);
  const error = new Error(`${prefix}: ${detail}`) as WireError;
  const code = safeIdentifier(metadata.code);
  const type = safeIdentifier(metadata.type);
  if (code !== undefined) error.code = code;
  if (type !== undefined) error.type = type;
  return error;
}

export function normalizeProviderError(providerId: string, error: unknown): ProviderRequestError {
  if (error instanceof ProviderRequestError) return error;
  const source = error instanceof Error ? error : new Error(String(error));
  return new ProviderRequestError(providerId, source, providerFailureDetails(providerId, source));
}

/** Keep caller cancellation intact; normalize every other adapter failure. */
export function throwProviderError(
  providerId: string,
  signal: AbortSignal | undefined,
  error: unknown,
): never {
  if (signal?.aborted === true) throw error;
  throw normalizeProviderError(providerId, error);
}

export function providerFailureDetails(
  providerId: string,
  error: Error,
): ProviderFailureDetails {
  if (error instanceof ProviderRequestError) {
    return compact({
      kind: error.kind,
      status: error.status,
      code: error.code,
      retryAfterMs: error.retryAfterMs,
      requestId: error.requestId,
    });
  }

  const http = error as HttpError;
  const wire = error as WireError;
  const body = bodyError(http.body);
  const code = firstIdentifier(wire.code, body.code);
  const type = firstIdentifier(wire.type, body.type);
  const evidence = [code, type, leadingText(error.message, 4_000), body.message]
    .filter((value): value is string => value !== undefined)
    .join(" ")
    .toLocaleLowerCase();
  const status = number(http.status);

  const kind = classify(providerId, status, evidence);
  return compact({
    kind,
    status,
    code,
    retryAfterMs: number(http.retryAfterMs),
    requestId: safeRequestId(http.requestId),
  });
}

/** Generation replay is allowed only for a definite transient rate rejection. */
export function isRetryableGenerationFailure(providerId: string, error: HttpError): boolean {
  return providerFailureDetails(providerId, error).kind === "rate-limit";
}

/** Idempotent reads may retry transient pressure, never account hard stops. */
export function isRetryableReadFailure(providerId: string, error: HttpError): boolean {
  const kind = providerFailureDetails(providerId, error).kind;
  return kind === "rate-limit" || kind === "overload" || kind === "network" || kind === "unknown";
}

function classify(
  providerId: string,
  status: number | undefined,
  evidence: string,
): ProviderFailureKind {
  if (
    providerId === "anthropic" &&
    /enforced[_ -]?spend[_ -]?limit[_ -]?reached/u.test(evidence)
  ) return "billing";
  if (
    (providerId === "openai" || providerId === "openai-codex") &&
    /billing[_ -]?hard[_ -]?limit[_ -]?reached/u.test(evidence)
  ) return "billing";
  if (
    providerId === "openai-codex" &&
    /usage[_ -]?limit[_ -]?reached|plan limit/u.test(evidence)
  ) return "quota";
  if (/insufficient[_ -]?quota|usage quota|quota (?:exceeded|exhausted)/u.test(evidence)) {
    return "quota";
  }
  if (
    status === 402 ||
    /\b(?:no|zero) credits? remaining\b|\bcredits? exhausted\b|billing|payment required|(?:hard|spend)[_ -]?limit/u
      .test(evidence)
  ) return "billing";

  if (
    status === 401 ||
    /invalid[_ -]?(?:api[_ -]?)?key|authentication|unauthorized|invalid[_ -]?token|oauth/u
      .test(evidence)
  ) return "authentication";

  if (
    /context[_ -]?(?:length|window)|maximum context|too many input tokens|prompt (?:is )?too long/u
      .test(evidence)
  ) return "context";

  if (
    status === 429 ||
    /rate[_ -]?limit|too many requests|tokens per min|requests per min|\btpm\b|\brpm\b/u
      .test(evidence)
  ) return "rate-limit";

  if (
    status === 408 ||
    status === 409 ||
    status === 500 ||
    status === 502 ||
    status === 503 ||
    status === 504 ||
    status === 529 ||
    /overload|temporarily unavailable|server is busy/u.test(evidence)
  ) return "overload";

  if (
    status === undefined &&
    /network error calling|timed out waiting|response body was idle|stream was idle|fetch failed|socket/u
      .test(evidence)
  ) return "network";

  return "unknown";
}

function bodyError(body: string | undefined): {
  code?: string;
  type?: string;
  message?: string;
} {
  if (body === undefined || body.trim() === "") return {};
  let parsed: unknown;
  try {
    parsed = JSON.parse(body) as unknown;
  } catch {
    return { message: body };
  }
  if (!record(parsed)) return {};

  const nested = record(parsed["error"]) ? parsed["error"] : undefined;
  const detail = record(parsed["detail"]) ? parsed["detail"] : undefined;
  return compact({
    code: firstText(nested?.["code"], detail?.["code"], parsed["code"]),
    type: firstText(nested?.["type"], detail?.["type"], parsed["type"]),
    message: firstText(
      nested?.["message"],
      typeof parsed["error"] === "string" ? parsed["error"] : undefined,
      detail?.["message"],
      typeof parsed["detail"] === "string" ? parsed["detail"] : undefined,
      parsed["message"],
    ),
  });
}

function firstText(...values: unknown[]): string | undefined {
  return values.find((value): value is string => typeof value === "string" && value !== "");
}

function firstIdentifier(...values: unknown[]): string | undefined {
  for (const value of values) {
    const identifier = safeIdentifier(value);
    if (identifier !== undefined) return identifier;
  }
  return undefined;
}

function safeIdentifier(value: unknown): string | undefined {
  return typeof value === "string" && /^[A-Za-z0-9._:-]{1,128}$/u.test(value)
    ? value
    : undefined;
}

function number(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function safeRequestId(value: unknown): string | undefined {
  return typeof value === "string" && /^[A-Za-z0-9._:-]{1,256}$/u.test(value)
    ? value
    : undefined;
}

function compact<T extends Record<string, unknown>>(value: T): T {
  return Object.fromEntries(
    Object.entries(value).filter(([, entry]) => entry !== undefined),
  ) as T;
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
