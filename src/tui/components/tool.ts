// A compact execution trace: state and identity on one rail, evidence below.

import type { Palette, RGB } from "../../ui/theme.ts";
import type { Seg } from "../../ui/render.ts";
import { hasColor, row } from "../../ui/render.ts";
import type { Detail, Emphasis, ToolBlock, ToolTone } from "./types.ts";

const OUTPUT_ROWS = 8;
const LIVE_OUTPUT_ROWS = 6;
const DIFF_ROWS = 15;

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
        ...(block.target === "" ? [] : [{ text: `  ${block.target}`, fg: pal.technical }]),
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
  // The compact transcript is an audit of what changed, not a code excerpt.
  // One shared budget applies to writes and edits. Keep both ends so a large
  // replacement cannot show only deletions while hiding all new content.
  // Context, omitted changes, and gap rows remain in semantic state for the
  // explicit full view.
  const changed = all.filter((detail) => detail.kind === "add" || detail.kind === "del");
  if (changed.length <= DIFF_ROWS) return changed;
  const leading = Math.ceil(DIFF_ROWS / 2);
  const trailing = DIFF_ROWS - leading;
  const hidden = changed.length - DIFF_ROWS;
  return [
    ...changed.slice(0, leading),
    {
      kind: "gap",
      text: `… ${hidden} more changed ${hidden === 1 ? "line" : "lines"} · ctrl+o expand`,
    },
    ...changed.slice(-trailing),
  ];
}

function renderDetail(detail: Detail, tone: ToolTone, width: number, pal: Palette): string {
  const lead: Seg = { text: " " };
  const rail: Seg = { text: "│ ", fg: pal.rule };
  if (detail.kind === "out") {
    const fg = tone === "fail" || failureLine(detail.text) ? pal.ink.removed : pal.ink.muted;
    return row(width, [lead, rail, { text: detail.text === "" ? " " : detail.text, fg }]);
  }
  if (detail.kind === "gap") {
    return row(width, [lead, rail, { text: detail.text, fg: pal.ink.dim, italic: true }]);
  }

  const number = detail.kind === "add" ? detail.newLine : detail.oldLine;
  const prefix = `${detail.kind === "add" ? "+" : detail.kind === "del" ? "-" : " "}${String(number ?? "").padStart(3)} `;
  const fg = detail.kind === "add"
    ? pal.ink.added
    : detail.kind === "del"
      ? pal.ink.removed
      : pal.ink.dim;
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
  return pal.ink.added;
}

function failureLine(line: string): boolean {
  return line.startsWith("✖") || line.startsWith("×") || line.includes("AssertionError") || /\bfailed\b/i.test(line);
}
