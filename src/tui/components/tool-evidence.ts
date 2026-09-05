// Selection and rendering of retained tool evidence. Never mutates its source.

import type { Palette, RGB } from "../../ui/theme.ts";
import type { Seg } from "../../ui/render.ts";
import { row } from "../../ui/render.ts";
import { graphemeCeiling, graphemeFloor } from "../../text-boundary.ts";
import { transcriptLead } from "../transcript-grammar.ts";
import type { Detail, Emphasis, ToolBlock, ToolTone } from "./types.ts";

export type ToolEvidence = { details: readonly Detail[]; counts: string; summary: string; note: string };

// Native exception headings may include a Node error code before the message.
const ERROR_DIAGNOSTIC = /^\s*\w*Error(?:\s+\[[^\]]+\])?\s*:|\berror\b\s*:|AssertionError/i;

export function toolEvidence(block: ToolBlock): ToolEvidence {
  const all = block.body ?? [];
  const changed: Detail[] = [];
  let added = 0, removed = 0, regions = 0, context = 3;
  for (const detail of all) {
    if (detail.kind === "keep") { context++; continue; }
    if (detail.kind !== "add" && detail.kind !== "del") { context = 3; continue; }
    if (context >= 3) regions++;
    context = 0;
    if (detail.kind === "add") added++;
    else removed++;
    changed.push(detail);
  }
  const counts = changed.length === 0 ? "" : `+${added} −${removed}`;
  const summary = counts === "" ? "" : `${counts} · ${regions} ${regions === 1 ? "region" : "regions"}`;
  if (all.length === 0) return { details: [], counts, summary, note: "" };
  if (block.expanded) return { details: all, counts, summary, note: "ctrl+o collapse" };
  if (changed.length > 0) return {
    details: changed.length <= 6 ? changed : [
      ...changed.slice(0, 3),
      { kind: "gap", text: `… ${changed.length - 6} more changed ${changed.length === 7 ? "line" : "lines"}` },
      ...changed.slice(-3),
    ], counts, summary, note: "ctrl+o full source",
  };
  const diagnostic = block.tone === "fail"
    ? all.find((detail) => ERROR_DIAGNOSTIC.test(detail.text))
      ?? all.find((detail) => /^not ok\b|^\s*[×✖]/i.test(detail.text)) : undefined;
  let details = all.slice(-4);
  if (diagnostic !== undefined && !details.includes(diagnostic)) details = [diagnostic, ...all.slice(-3)];
  const hidden = all.length - details.length;
  return {
    details, counts, summary,
    note: hidden > 0 ? `${hidden} ${diagnostic === undefined ? "earlier" : "other"} ${hidden === 1 ? "line" : "lines"} · ctrl+o`
      : `${all.length} ${all.length === 1 ? "line" : "lines"}`,
  };
}

export function renderDetail(
  detail: Detail,
  tone: ToolTone,
  width: number,
  pal: Palette,
  railInk: RGB = pal.rule,
): string {
  const rail = transcriptLead(width, { text: "│", fg: railInk });
  if (detail.kind === "out") {
    const base = tone === "fail" || failureLine(detail.text) ? pal.ink.removed : pal.ink.muted;
    return row(width, [
      ...rail,
      { text: detail.text === "" ? " " : detail.text, fg: base },
    ]);
  }
  if (detail.kind === "gap") {
    return row(width, [
      ...rail,
      { text: detail.text, fg: pal.ink.dim, italic: true },
    ]);
  }

  const number = detail.kind === "add" ? detail.newLine : detail.oldLine;
  const sign = detail.kind === "add" ? "+" : detail.kind === "del" ? "-" : " ";
  const fg = detail.kind === "add"
    ? pal.ink.added
    : detail.kind === "del"
      ? pal.ink.removed
      : pal.ink.dim;
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

function failureLine(line: string): boolean {
  return line.startsWith("✖") || line.startsWith("×") || line.includes("AssertionError") || /\bfailed\b/i.test(line);
}
