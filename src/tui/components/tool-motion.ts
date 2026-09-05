// One travelling light on the execution connector; evidence never animates.

import { hasColor } from "../../ui/render.ts";
import type { Seg } from "../../ui/render.ts";
import type { Palette, RGB } from "../../ui/theme.ts";
import type { Block, ToolBlock } from "./types.ts";

type Clock = { now?: number; reducedMotion?: boolean };

export function runningTool(block: Block): block is ToolBlock & { tone: "pending"; startedAt: number; right: "running" } {
  return block.kind === "tool" && block.tone === "pending" &&
    block.startedAt !== undefined && block.right === "running";
}

export function stateInk(block: ToolBlock, pal: Palette): RGB {
  if (block.tone === "fail") return pal.ink.removed;
  if (block.tone === "deny") return pal.ink.attention;
  if (block.tone === "ok") return pal.ink.added;
  return runningTool(block) ? pal.accent : pal.ink.dim;
}

export function stateMark(block: ToolBlock): string {
  return block.tone === "ok" ? "✓" : block.tone === "fail" ? "×" : block.tone === "deny" ? "!" : "○";
}

export function connectorInk(
  block: ToolBlock, pal: Palette, context: Clock, position: number, length: number,
): RGB {
  if (context.reducedMotion === true || block.expanded === true || !hasColor() || !runningTool(block)) return pal.rule;
  const elapsed = Math.max(0, (context.now ?? Date.now()) - block.startedAt!);
  const head = (elapsed % 2000) / 2000 * (length + 3) - 1.5;
  const amount = Math.max(0, 1 - Math.abs(position - head) / 1.8);
  const channel = (index: 0 | 1 | 2) => Math.round(pal.rule[index] + (pal.accent[index] - pal.rule[index]) * amount);
  return [channel(0), channel(1), channel(2)];
}

export function connector(
  text: string, block: ToolBlock, pal: Palette, context: Clock, position: number, length: number,
): Seg[] {
  return Array.from(text, (char, index) => ({
    text: char, fg: connectorInk(block, pal, context, position + index / 3, length),
  }));
}
