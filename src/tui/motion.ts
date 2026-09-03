// Small deterministic motion primitives for terminal cells.

import type { RGB } from "../ui/theme.ts";

export const TOOL_BIRTH_MS = 300;
export const TOOL_SETTLE_MS = 700;
export const TOOL_ROW_ARRIVAL_MS = 420;
export const TOOL_LEADER_MAX_MS = 1_500;

export type ToolMotion = Readonly<{
  bornAt: number;
  settledAt?: number;
  rowsAt: readonly number[];
}>;

export function interval(now: number, start: number | undefined, duration: number): number {
  if (start === undefined) return 1;
  if (duration <= 0) return 1;
  return Math.max(0, Math.min(1, (now - start) / duration));
}

export function easeOut(value: number): number {
  const rest = 1 - Math.max(0, Math.min(1, value));
  return 1 - rest * rest * rest;
}

export function easeInOut(value: number): number {
  const at = Math.max(0, Math.min(1, value));
  return at < 0.5 ? 4 * at * at * at : 1 - Math.pow(-2 * at + 2, 3) / 2;
}

export function mix(from: RGB, to: RGB, amount: number): RGB {
  const at = Math.max(0, Math.min(1, amount));
  return [
    Math.round(from[0] + (to[0] - from[0]) * at),
    Math.round(from[1] + (to[1] - from[1]) * at),
    Math.round(from[2] + (to[2] - from[2]) * at),
  ];
}

/** A restrained pulse that never reaches either endpoint. */
export function breathe(now: number, period = 1_400): number {
  return 0.2 + ((Math.sin((now / period) * Math.PI * 2) + 1) / 2) * 0.6;
}
