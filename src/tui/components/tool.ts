// A compact execution trace: state and identity on one rail, evidence below.

import type { Palette, RGB } from "../../ui/theme.ts";
import type { Seg } from "../../ui/render.ts";
import { hasColor, row } from "../../ui/render.ts";
import type { Detail, Emphasis, ToolBlock, ToolTone } from "./types.ts";

const OUTPUT_ROWS = 8;
const LIVE_OUTPUT_ROWS = 6;
const DIFF_ROWS = 12;

export type ToolRenderContext = {
  continues?: boolean;
  spin?: number;
  reducedMotion?: boolean;
  now?: number;
};

export function renderTool(
  block: ToolBlock,
  width: number,
  pal: Palette,
  context: ToolRenderContext = {},
): string[] {
  const shown = visibleDetails(block);
  const right = liveLabel(block, context);
  return [
    ...(context.continues === true ? [] : [""]),
    row(
      width,
      [
        { text: " " },
        { text: `${stateGlyph(block, context)} `, fg: statusInk(block.tone, pal), bold: true },
        { text: block.name, fg: pal.ink.bright, bold: true },
        ...(block.target === "" ? [] : [{ text: `  ${block.target}`, fg: pal.accent }]),
      ],
      right === "" ? [] : [{ text: right, fg: statusInk(block.tone, pal) }],
    ),
    ...shown.map((detail) => renderDetail(detail, block.tone, width, pal)),
  ];
}

function visibleDetails(block: ToolBlock): Detail[] {
  const all = block.body ?? [];
  if (block.expanded === true || all.length === 0) return all;
  if (all.every((detail) => detail.kind === "out")) {
    const limit = block.tone === "pending" ? LIVE_OUTPUT_ROWS : OUTPUT_ROWS;
    if (all.length <= limit) return all;
    const hidden = all.length - limit;
    const note = block.tone === "pending"
      ? `… ${all.length} lines so far`
      : `… ${hidden} earlier lines · ctrl+o expand`;
    return [{ kind: "gap", text: note }, ...all.slice(-limit)];
  }
  return diffWindow(all);
}

/** Keep change rows and their nearest context; never turn a diff into a tail. */
function diffWindow(all: readonly Detail[]): Detail[] {
  if (all.length <= DIFF_ROWS) return [...all];

  const important = all.flatMap((detail, index) => detail.kind === "keep" ? [] : [index]);
  const chosen = new Set<number>();
  const budgeted = important.length <= DIFF_ROWS
    ? important
    : [
        ...important.slice(0, Math.ceil(DIFF_ROWS / 2)),
        ...important.slice(-Math.floor(DIFF_ROWS / 2)),
      ];
  for (const index of budgeted) chosen.add(index);

  for (let radius = 1; chosen.size < DIFF_ROWS && radius < all.length; radius++) {
    for (const index of important) {
      for (const candidate of [index - radius, index + radius]) {
        if (candidate < 0 || candidate >= all.length || chosen.has(candidate)) continue;
        chosen.add(candidate);
        if (chosen.size >= DIFF_ROWS) break;
      }
      if (chosen.size >= DIFF_ROWS) break;
    }
  }

  const indexes = [...chosen].sort((left, right) => left - right);
  const out: Detail[] = [];
  let previous = -1;
  for (const index of indexes) {
    if (index > previous + 1) {
      out.push({ kind: "gap", text: `… ${index - previous - 1} lines hidden · ctrl+o expand` });
    }
    const detail = all[index];
    if (detail !== undefined) out.push(detail);
    previous = index;
  }
  if (previous < all.length - 1) {
    out.push({
      kind: "gap",
      text: `… ${all.length - previous - 1} lines hidden · ctrl+o expand`,
    });
  }
  return out;
}

function renderDetail(detail: Detail, tone: ToolTone, width: number, pal: Palette): string {
  const lead: Seg = { text: " " };
  const rail: Seg = { text: "│ ", fg: pal.rule };
  if (detail.kind === "out") {
    const fg = tone === "fail" || failureLine(detail.text) ? pal.ink.removed : pal.ink.muted;
    return row(width, [lead, rail, { text: detail.text === "" ? " " : detail.text, fg }]);
  }
  if (detail.kind === "gap") {
    return row(width, [lead, rail, { text: detail.text, fg: pal.ink.muted, italic: true }]);
  }

  const number = detail.kind === "add" ? detail.newLine : detail.oldLine;
  const prefix = `${detail.kind === "add" ? "+" : detail.kind === "del" ? "-" : " "}${String(number ?? "").padStart(3)} `;
  const fg = detail.kind === "add"
    ? pal.ink.added
    : detail.kind === "del"
      ? pal.ink.removed
      : pal.ink.muted;
  return row(width, [lead, rail, { text: prefix, fg }, ...emphasized(detail.text, detail.emphasis, fg)]);
}

function emphasized(text: string, emphasis: Emphasis | undefined, fg: RGB): Seg[] {
  if (emphasis === undefined || emphasis.length <= 0) return [{ text, fg }];
  const start = Math.max(0, Math.min(text.length, emphasis.start));
  const end = Math.max(start, Math.min(text.length, start + emphasis.length));
  return [
    ...(start === 0 ? [] : [{ text: text.slice(0, start), fg }]),
    { text: text.slice(start, end), fg, inverse: true },
    ...(end === text.length ? [] : [{ text: text.slice(end), fg }]),
  ];
}

function stateGlyph(block: ToolBlock, context: ToolRenderContext): string {
  if (block.tone === "pending") {
    if (block.startedAt === undefined || context.reducedMotion === true) return "◌";
    const frames = ["◐", "◓", "◑", "◒"] as const;
    return frames[Math.floor((context.spin ?? 0) / 2) % frames.length] as string;
  }
  if (block.tone === "fail") return hasColor() ? "●" : "×";
  if (block.tone === "deny") return "○";
  return hasColor() ? "●" : "✓";
}

function liveLabel(block: ToolBlock, context: ToolRenderContext): string {
  if (block.tone !== "pending" || block.startedAt === undefined || block.right !== "running") {
    return block.right;
  }
  const elapsed = Math.max(0, (context.now ?? Date.now()) - block.startedAt);
  const seconds = elapsed / 1_000;
  return `running · ${seconds < 10 ? seconds.toFixed(1) : Math.round(seconds)}s`;
}

function statusInk(tone: ToolTone, pal: Palette): RGB {
  if (tone === "fail") return pal.ink.removed;
  if (tone === "pending" || tone === "deny") return pal.ink.attention;
  return pal.ink.muted;
}

function failureLine(line: string): boolean {
  return line.startsWith("✖") || line.startsWith("×") || line.includes("AssertionError") || /\bfailed\b/i.test(line);
}
