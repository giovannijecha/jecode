// Shared strict value checks and limits for the canonical session codec.

import { Buffer } from "node:buffer";
import { MAX_TEXT_CODE_UNITS } from "../text-boundary.ts";

export const SESSION_FILE_LIMITS = Object.freeze({
  text: MAX_TEXT_CODE_UNITS,
  metadataBytes: 64 * 1_024,
  nodeBytes: 20 * 1_024 * 1_024,
  jsonDepth: 24,
  jsonNodes: 32_768,
  blocks: 8_192,
  details: 8_192,
});

function line(value: unknown): string {
  return `${JSON.stringify(value, null, 2)}\n`;
}

export function boundedLine(value: unknown, maxBytes: number): string {
  const encoded = line(value);
  if (Buffer.byteLength(encoded, "utf8") > maxBytes) throw invalid();
  return encoded;
}

export function keys(value: Record<string, unknown>, expected: string): boolean {
  return Object.keys(value).sort().join(",") === expected;
}

export function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function bounded(value: unknown, limit: number = SESSION_FILE_LIMITS.text): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= limit;
}

export function boundedText(value: unknown, limit: number = SESSION_FILE_LIMITS.text): value is string {
  return typeof value === "string" && value.length <= limit;
}

export function integer(value: unknown, minimum: number): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= minimum;
}

export function nullableInteger(value: unknown, minimum: number): value is number | null {
  return value === null || integer(value, minimum);
}

export function identifier(value: unknown): value is string {
  return typeof value === "string" && /^[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}$/.test(value);
}

export function digest(value: unknown): value is string {
  return typeof value === "string" && /^[a-f0-9]{64}$/.test(value);
}

export function timestamp(value: unknown): value is string {
  return typeof value === "string" && value.length >= 20 && value.length <= 64 &&
    Number.isFinite(Date.parse(value));
}

export function invalid(): Error {
  return new Error("session data is invalid or unsupported");
}
