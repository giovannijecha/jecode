// Ribbon rows and bounded overflow recovery shared by completion and selectors.

import type { Palette } from "../../ui/theme.ts";
import { row } from "../../ui/render.ts";
import { terminalText } from "../../ui/terminal-text.ts";
import { elide, textWidth, wrapText } from "../../ui/width.ts";

export type MenuEntry = {
  label: string;
  hint?: string;
  value?: string;
  adjustable?: boolean;
  selected: boolean;
};

export function renderMenuRows(entries: readonly MenuEntry[], width: number, pal: Palette): string[] {
  return entries.map((entry) => {
    const right = summary(entry, width);
    return row(width, [
      { text: entry.selected ? "● " : "  ", fg: pal.focus },
      { text: entry.label, fg: entry.selected ? pal.ink.bright : pal.ink.muted, bold: entry.selected },
    ], right === "" ? [] : [{ text: right, fg: entry.selected ? pal.focus : pal.ink.dim }],
    entry.selected ? pal.surface.subtle : undefined);
  });
}

/** A missing palette measures the window without producing styled rows. */
export function renderMenu(
  entries: readonly MenuEntry[], width: number, pal: Palette | undefined,
  options: { maxRows: number; visible?: number },
): { rows: string[]; first: number; last: number } {
  const room = Math.max(0, options.maxRows);
  if (room === 0) return { rows: [], first: 0, last: 0 };
  const detailLimit = Math.min(2, room - 1);
  let detailRows = 0;
  // Measure every possible selection so revealing clipped identity and values
  // does not move the dock when the selected row changes.
  for (const entry of entries) {
    detailRows = Math.max(detailRows, detailLines(entry, width, detailLimit).length);
    if (detailRows === detailLimit) break;
  }
  const visible = Math.min(options.visible ?? 6, room - detailRows);
  const at = Math.max(0, entries.findIndex((entry) => entry.selected));
  const { first, last } = menuWindow(entries.length, at, visible);
  if (pal === undefined) return { rows: [], first, last };
  if (entries.length === 0) return {
    rows: [row(width, [{ text: "No matches. Change the filter.", fg: pal.ink.muted }])], first, last,
  };
  const active = entries.find((entry) => entry.selected);
  const lines = detailLines(active, width, detailRows);
  return {
    rows: [
      ...renderMenuRows(entries.slice(first, last), width, pal),
      ...Array.from({ length: detailRows }, (_, index) => lines[index] === undefined ? ""
        : row(width, [{ text: "  " + lines[index], fg: pal.ink.muted }])),
    ], first, last,
  };
}

function detailLines(entry: MenuEntry | undefined, width: number, maxRows: number): string[] {
  if (entry === undefined || maxRows <= 0) return [];
  const room = Math.max(1, width - 2);
  return menuText(clippedParts(entry, width).join(" · "), room, maxRows);
}

export function menuWindow(length: number, selected: number, visible: number): { first: number; last: number } {
  const count = Math.max(0, length);
  const room = Math.max(1, visible);
  const at = count === 0 ? 0 : Math.max(0, Math.min(count - 1, selected));
  const first = Math.max(0, Math.min(at - Math.floor(room / 2), count - room));
  return { first, last: Math.min(count, first + room) };
}

/** Wrap untrusted menu copy, marking a bounded final row when it omits text. */
export function menuText(text: string, width: number, maxRows: number): string[] {
  if (text === "" || maxRows <= 0) return [];
  const lines = wrapText(terminalText(text), Math.max(1, width));
  return lines.slice(0, maxRows).map((line, index) =>
    index === maxRows - 1 && lines.length > maxRows ? elide(line + " …", width) : line);
}

function fullSummary(entry: MenuEntry): string {
  return [entry.hint, displayedValue(entry)].filter(Boolean).join(" · ");
}

function summary(entry: MenuEntry, width: number): string {
  const room = Math.max(1, Math.floor(width * 0.42));
  const value = terminalText(displayedValue(entry));
  const hint = terminalText(entry.hint ?? "");
  if (value === "" || hint === "") return elide(value || hint, room);
  // Keep the current policy visible even when its remembered-scope hint is long.
  const hintRoom = room - textWidth(value) - 3;
  return hintRoom <= 0 ? elide(value, room) : `${elide(hint, hintRoom)} · ${value}`;
}

function displayedValue(entry: MenuEntry): string {
  if (entry.value === undefined) return "";
  return entry.selected && entry.adjustable ? `‹ ${entry.value} ›` : entry.value;
}

function clippedParts(entry: MenuEntry, width: number): string[] {
  // Reserve the stepper's width for every peer, so moving selection cannot
  // make the shared detail area appear or disappear.
  const stable = entry.adjustable ? { ...entry, selected: true } : entry;
  const right = summary(stable, width);
  const leftRoom = Math.max(0, width - 2 - (right === "" ? 0 : textWidth(right) + 1));
  return [
    ...(textWidth(terminalText(entry.label)) > leftRoom ? [entry.label] : []),
    ...(right !== terminalText(fullSummary(stable)) ? [fullSummary(stable)] : []),
  ];
}
