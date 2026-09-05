// Target-first execution records, connected to bounded, expandable evidence.

import type { Palette } from "../../ui/theme.ts";
import type { Seg } from "../../ui/render.ts";
import { plainLen, row } from "../../ui/render.ts";
import { toolDuration } from "../../duration.ts";
import type { ToolBlock } from "./types.ts";
import { toolEvidence, renderDetail } from "./tool-evidence.ts";
import { connector, connectorInk, runningTool, stateInk, stateMark } from "./tool-motion.ts";

export type ToolRenderContext = { continues?: boolean; now?: number; reducedMotion?: boolean };

export function renderTool(
  block: ToolBlock, width: number, pal: Palette, context: ToolRenderContext = {},
): string[] {
  const now = context.now ?? Date.now();
  const clock = { ...context, now };
  const evidence = toolEvidence(block);
  const active = runningTool(block);
  const elapsed = active ? toolDuration(Math.max(0, now - block.startedAt!), true)
    : block.durationMs === undefined ? "" : toolDuration(block.durationMs);
  const mark = stateMark(block);
  // The separate change summary already carries these exact counts.
  const repeatedCount = evidence.counts !== "" && block.right.startsWith(`${evidence.counts} · `);
  const outcome = repeatedCount ? block.right.slice(evidence.counts.length + 3) : block.right;
  const result = [`${mark}${outcome === "" ? "" : ` ${outcome}`}`, elapsed].filter(Boolean).join(" · ");
  const hasEvidence = evidence.details.length > 0;
  const name: Seg = { text: block.name, fg: pal.ink.dim };
  const right: Seg[] = [{ text: result, fg: stateInk(block, pal) }];
  const separateStatus = plainLen([name, ...right]) + (hasEvidence ? 2 : 3) + 3 > width;
  const summaryAt = separateStatus ? 3 : 2;
  const detailStart = summaryAt + (evidence.summary === "" ? 0 : 1);
  const length = hasEvidence ? detailStart + evidence.details.length + 1 : summaryAt;
  const left: Seg[] = [
    ...connector(hasEvidence || separateStatus ? "│ " : "└─ ", block, pal, clock, 1, length),
    name,
  ];
  const rows = [
    ...(context.continues === true ? [] : [""]),
    row(width, [
      ...connector("┌ ", block, pal, clock, 0, length),
      { text: block.target || block.name, fg: pal.technical },
    ]),
    separateStatus ? row(width, left)
      : row(Math.min(width, plainLen([...left, ...right]) + 3), left, right),
  ];
  if (separateStatus) rows.push(row(width, [
    ...connector(hasEvidence ? "│ " : "└─ ", block, pal, clock, 2, length),
    ...right,
  ]));
  if (!hasEvidence) return rows;
  if (evidence.summary !== "") rows.push(row(width, [
    ...connector("│ ", block, pal, clock, summaryAt, length),
    { text: evidence.summary, fg: pal.ink.muted },
  ]));
  rows.push(...evidence.details.map((detail, index) => renderDetail(
    detail, block.tone, width, pal,
    block.expanded ? pal.rule : connectorInk(block, pal, clock, detailStart + index, length),
  )));
  rows.push(row(width, [
    ...connector("└─", block, pal, clock, length - 1, length),
    { text: evidence.note === "" ? "" : ` ${evidence.note}`, fg: pal.ink.dim },
  ]));
  return rows;
}
