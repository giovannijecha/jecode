// A compact execution trace: state and identity on one semantic rail,
// bounded evidence below, and motion that never enters durable state.

import type { Palette, RGB } from "../../ui/theme.ts";
import type { Seg } from "../../ui/render.ts";
import { fitSegs, hasColor, plainLen, row } from "../../ui/render.ts";
import { graphemeCeiling, graphemeFloor } from "../../text-boundary.ts";
import { toolDuration } from "../../duration.ts";
import {
  breathe,
  easeInOut,
  easeOut,
  interval,
  mix,
  TOOL_BIRTH_MS,
  TOOL_LEADER_MAX_MS,
  TOOL_ROW_ARRIVAL_MS,
  type ToolMotion,
} from "../motion.ts";
import { transcriptLead, transcriptMark } from "../transcript-grammar.ts";
import type { Detail, Emphasis, ToolBlock, ToolTone } from "./types.ts";

const OUTPUT_ROWS = 8;
const LIVE_OUTPUT_ROWS = 6;
const DIFF_ROWS = 15;
const TOOL_NAME_COLS = 12;
const TOOL_COLUMNS_AT = 64;
const SPINNER_MS = 80;
const SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"] as const;

type ShownDetail = { detail: Detail; sourceIndex?: number };

export type ToolRenderContext = {
  continues?: boolean;
  followsReasoning?: boolean;
  now?: number;
  motion?: ToolMotion;
  reducedMotion?: boolean;
};

export function renderTool(
  block: ToolBlock,
  width: number,
  pal: Palette,
  context: ToolRenderContext = {},
): string[] {
  const now = context.now ?? Date.now();
  const shown = visibleDetails(block);
  const ink = stateInk(block, pal, context, now);
  const nameInk = birthInk(pal.ink.bright, pal.ink.dim, context, now);
  const left: Seg[] = [
    ...transcriptLead(width, { text: stateGlyph(block, context, now), fg: ink, bold: true }),
    { text: toolName(block, width), fg: nameInk, bold: true },
    ...(block.target === "" ? [] : [{ text: `  ${block.target}`, fg: pal.technical }]),
  ];
  const right = resultSegments(block, pal, context, now);
  const leader = movingLeader(width, left, right, pal, context, now);

  return [
    ...(context.followsReasoning === true
      ? [row(width, transcriptMark(width, { text: "│", fg: pal.rule }))]
      : context.continues === true ? [] : [""]),
    row(width, leader === undefined ? left : [...left, leader], right),
    ...shown.map(({ detail, sourceIndex }) => renderDetail(
      detail,
      block.tone,
      width,
      pal,
      context.reducedMotion === true ? undefined : context.motion?.rowsAt[sourceIndex ?? -1],
      now,
    )),
  ];
}

function visibleDetails(block: ToolBlock): ShownDetail[] {
  const all = block.body ?? [];
  if (block.expanded === true || all.length === 0) {
    return all.map((detail, sourceIndex) => ({ detail, sourceIndex }));
  }
  if (all.every((detail) => detail.kind === "out")) {
    const limit = block.tone === "pending" ? LIVE_OUTPUT_ROWS : OUTPUT_ROWS;
    if (all.length <= limit) return all.map((detail, sourceIndex) => ({ detail, sourceIndex }));
    const hidden = all.length - limit;
    const note = block.tone === "pending"
      ? `… ${all.length} lines so far`
      : `… ${hidden} earlier lines · ctrl+o expand`;
    return [
      { detail: { kind: "gap", text: note } },
      ...all.slice(-limit).map((detail, index) => ({
        detail,
        sourceIndex: all.length - limit + index,
      })),
    ];
  }

  // A compact transcript audits what changed rather than repeating unchanged
  // source. One budget covers writes and edits, with both ends retained.
  const changed = all
    .map((detail, sourceIndex) => ({ detail, sourceIndex }))
    .filter(({ detail }) => detail.kind === "add" || detail.kind === "del");
  if (changed.length <= DIFF_ROWS) return changed;
  const leading = Math.ceil(DIFF_ROWS / 2);
  const trailing = DIFF_ROWS - leading;
  const hidden = changed.length - DIFF_ROWS;
  return [
    ...changed.slice(0, leading),
    {
      detail: {
        kind: "gap",
        text: `… ${hidden} more changed ${hidden === 1 ? "line" : "lines"} · ctrl+o expand`,
      },
    },
    ...changed.slice(-trailing),
  ];
}

function renderDetail(
  detail: Detail,
  tone: ToolTone,
  width: number,
  pal: Palette,
  arrivedAt: number | undefined,
  now: number,
): string {
  const rail = transcriptLead(width, { text: "│", fg: pal.rule });
  if (detail.kind === "out") {
    const base = tone === "fail" || failureLine(detail.text) ? pal.ink.removed : pal.ink.muted;
    return row(width, [
      ...rail,
      { text: detail.text === "" ? " " : detail.text, fg: arrivalInk(base, pal, arrivedAt, now) },
    ]);
  }
  if (detail.kind === "gap") {
    return row(width, [
      ...rail,
      { text: detail.text, fg: arrivalInk(pal.ink.dim, pal, arrivedAt, now), italic: true },
    ]);
  }

  const number = detail.kind === "add" ? detail.newLine : detail.oldLine;
  const sign = detail.kind === "add" ? "+" : detail.kind === "del" ? "-" : " ";
  const base = detail.kind === "add"
    ? pal.ink.added
    : detail.kind === "del"
      ? pal.ink.removed
      : pal.ink.dim;
  const fg = arrivalInk(base, pal, arrivedAt, now);
  const tint = detail.kind === "add"
    ? pal.surface.added
    : detail.kind === "del"
      ? pal.surface.removed
      : undefined;
  return row(width, [
    ...rail,
    { text: sign, fg, bold: detail.kind !== "keep" },
    { text: `${String(number ?? "").padStart(4)} `, fg: pal.ink.dim },
    ...emphasized(detail.text, detail.emphasis, fg, tint),
  ]);
}

function emphasized(
  text: string,
  emphasis: Emphasis | undefined,
  fg: RGB,
  bg: RGB | undefined,
): Seg[] {
  if (emphasis === undefined || emphasis.length <= 0) return [{ text, fg }];
  const requestedStart = Math.max(0, Math.min(text.length, emphasis.start));
  const requestedEnd = Math.max(requestedStart, Math.min(text.length, requestedStart + emphasis.length));
  const start = graphemeFloor(text, requestedStart);
  const end = graphemeCeiling(text, requestedEnd);
  if (start === end) return [{ text, fg }];
  return [
    ...(start === 0 ? [] : [{ text: text.slice(0, start), fg }]),
    { text: text.slice(start, end), fg, bg, bold: true },
    ...(end === text.length ? [] : [{ text: text.slice(end), fg }]),
  ];
}

function toolName(block: ToolBlock, width: number): string {
  if (width < TOOL_COLUMNS_AT || block.target === "") return block.name;
  return block.name.padEnd(TOOL_NAME_COLS);
}

function stateGlyph(block: ToolBlock, context: ToolRenderContext, now: number): string {
  if (block.tone === "pending") {
    if (block.startedAt === undefined || block.right !== "running") return "○";
    if (context.reducedMotion === true) return "○";
    return SPINNER[Math.floor(now / SPINNER_MS) % SPINNER.length] ?? "○";
  }
  if (block.tone === "fail") return hasColor() ? "●" : "×";
  if (block.tone === "deny") return "○";
  return hasColor() ? "●" : "✓";
}

function resultSegments(
  block: ToolBlock,
  pal: Palette,
  context: ToolRenderContext,
  now: number,
): Seg[] {
  const status = block.right;
  const duration = liveDuration(block, now);
  if (status === "" && duration === "") return [];
  const ink = resultInk(block, pal, context, now);
  return [
    ...(status === "" ? [] : [{ text: status, fg: ink }]),
    ...(duration === "" ? [] : [{ text: `${status === "" ? "" : " · "}${duration}`, fg: pal.ink.dim }]),
  ];
}

function liveDuration(block: ToolBlock, now: number): string {
  if (block.tone === "pending" && block.startedAt !== undefined && block.right === "running") {
    return toolDuration(Math.max(0, now - block.startedAt), true);
  }
  return block.durationMs === undefined ? "" : toolDuration(block.durationMs);
}

function stateInk(
  block: ToolBlock,
  pal: Palette,
  context: ToolRenderContext,
  now: number,
): RGB {
  if (block.tone === "fail") return pal.ink.removed;
  if (block.tone === "deny") return pal.ink.attention;
  if (block.tone === "pending") {
    if (block.startedAt === undefined || context.reducedMotion === true) return pal.ink.attention;
    return mix(pal.ink.attention, pal.accent, breathe(now));
  }
  return pal.ink.added;
}

function resultInk(
  block: ToolBlock,
  pal: Palette,
  context: ToolRenderContext,
  now: number,
): RGB {
  return block.tone === "ok" ? pal.ink.muted : stateInk(block, pal, context, now);
}

function birthInk(final: RGB, initial: RGB, context: ToolRenderContext, now: number): RGB {
  if (context.reducedMotion === true || context.motion === undefined) return final;
  return mix(initial, final, easeOut(interval(now, context.motion.bornAt, TOOL_BIRTH_MS)));
}

function arrivalInk(base: RGB, pal: Palette, arrivedAt: number | undefined, now: number): RGB {
  if (arrivedAt === undefined) return base;
  return mix(pal.ink.bright, base, easeOut(interval(now, arrivedAt, TOOL_ROW_ARRIVAL_MS)));
}

function movingLeader(
  width: number,
  left: readonly Seg[],
  right: readonly Seg[],
  pal: Palette,
  context: ToolRenderContext,
  now: number,
): Seg | undefined {
  if (context.reducedMotion === true || context.motion === undefined || right.length === 0) return undefined;
  const progress = interval(now, context.motion.bornAt, TOOL_LEADER_MAX_MS);
  if (progress >= 1) return undefined;

  const fittedRight = fitSegs(right, width);
  const gap = width - plainLen(left) - plainLen(fittedRight) - 1;
  if (gap < 9) return undefined;

  const cells = Array.from({ length: gap }, () => " ");
  const travelCells = gap - 2;
  const trail = Math.min(7, travelCells);
  const travel = travelCells + trail;
  const head = Math.floor(easeInOut(progress) * travel) - trail;
  for (let offset = 0; offset < trail; offset++) {
    const at = head + offset;
    if (at >= 0 && at < travelCells) cells[at + 1] = "·";
  }
  return { text: cells.join(""), fg: pal.rule };
}

function failureLine(line: string): boolean {
  return line.startsWith("✖") || line.startsWith("×") || line.includes("AssertionError") || /\bfailed\b/i.test(line);
}
