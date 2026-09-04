// Actionable provider failures for user-facing command and turn surfaces.

import type { Provider } from "./types.ts";
import { redactCredentials } from "./credential-safety.ts";
import { providerLabel } from "./provider-label.ts";
import { leadingText } from "./text-boundary.ts";
import { terminalText } from "./ui/terminal-text.ts";
import type { HttpError } from "./providers/http.ts";

const CONNECTION_FAILURE =
  /network error calling|timed out waiting for response headers|response body was idle/i;
const MAX_MESSAGE_CHARS = 1_000;
const MAX_REASON_CHARS = 500;
const HTML = /<!doctype\s|<\/?[a-z][^>]*>/i;

export function providerFailure(
  provider: Provider,
  error: Error,
  labelProvider = false,
): string {
  if (provider.id === "ollama" && CONNECTION_FAILURE.test(error.message)) {
    return provider.location?.() === "local"
      ? "Ollama is not reachable on this computer · start Ollama or choose cloud in /providers"
      : "Ollama is not reachable · check its connection in /providers";
  }
  const message = safeText(error.message, MAX_MESSAGE_CHARS) || "provider request failed";
  const reason = providerReason(error);
  const detail = reason !== undefined && !message.toLocaleLowerCase().includes(reason.toLocaleLowerCase())
    ? ` · ${reason}`
    : "";
  const failure = `${message}${detail}`;
  return labelProvider ? `${providerLabel(provider.id)}: ${failure}` : failure;
}

function providerReason(error: Error): string | undefined {
  const body = (error as HttpError).body;
  if (typeof body !== "string" || body.trim() === "") return undefined;

  let parsed: unknown;
  try {
    parsed = JSON.parse(body) as unknown;
  } catch {
    const raw = body.trim();
    if (raw.startsWith("{") || raw.startsWith("[") || HTML.test(raw)) return undefined;
    return safeReason(raw);
  }

  for (const candidate of reasonCandidates(parsed)) {
    const reason = safeReason(candidate);
    if (reason !== undefined) return reason;
  }
  return undefined;
}

function reasonCandidates(value: unknown): string[] {
  if (typeof value === "string") return [value];
  if (!record(value)) return [];
  const error = value["error"];
  return [
    record(error) ? error["message"] : undefined,
    typeof error === "string" ? error : undefined,
    value["message"],
    value["detail"],
  ].filter((candidate): candidate is string => typeof candidate === "string");
}

function safeReason(text: string): string | undefined {
  if (HTML.test(text)) return undefined;
  return safeText(text, MAX_REASON_CHARS) || undefined;
}

function safeText(text: string, max: number): string {
  const redacted = redactCredentials(text).trim().replace(/\s+/gu, " ");
  return leadingText(terminalText(redacted), max);
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
