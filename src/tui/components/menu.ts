// One visual row grammar for autocomplete and every interactive selector.

import type { Palette } from "../../ui/theme.ts";
import type { Seg } from "../../ui/render.ts";
import { hasColor, plainLen, row } from "../../ui/render.ts";
import { elide } from "../../ui/width.ts";

export type MenuEntry = {
  label: string;
  description?: string;
  hint?: string;
  value?: string;
  adjustable?: boolean;
  selected: boolean;
};

export function renderMenuRows(entries: readonly MenuEntry[], width: number, pal: Palette): string[] {
  if (entries.length === 0) return [];
  const widest = Math.max(...entries.map(primaryWidth));
  const labelWidth = Math.min(42, Math.max(12, widest + 2));
  return entries.map((entry) => renderEntry(entry, labelWidth, width, pal));
}

export function menuWindow(length: number, selected: number, visible: number): { first: number; last: number } {
  const count = Math.max(0, length);
  const room = Math.max(1, visible);
  const at = count === 0 ? 0 : Math.max(0, Math.min(count - 1, selected));
  const first = Math.max(0, Math.min(at - Math.floor(room / 2), count - room));
  return { first, last: Math.min(count, first + room) };
}

function renderEntry(entry: MenuEntry, labelWidth: number, width: number, pal: Palette): string {
  // Colour terminals spend focus on the active label instead of painting a
  // full-width band. Monochrome has no colour, so it alone reserves a fixed
  // arrow column to keep selection visible without shifting peer rows.
  const monochrome = !hasColor();
  const selectedMark = monochrome ? (entry.selected ? "→ " : "  ") : "";
  const fg = entry.selected ? pal.focus : pal.ink.fg;
  const primary: Seg[] = [
    { text: selectedMark, fg },
    { text: entry.label, fg, bold: entry.selected || undefined },
  ];

  if (width > 40 && entry.description !== undefined) {
    const gap = Math.max(2, labelWidth - primaryWidth(entry));
    primary.push({
      text: `${" ".repeat(gap)}${entry.description}`,
      fg: entry.selected ? pal.ink.bright : pal.ink.muted,
    });
  }

  const value = entry.value === undefined
    ? undefined
    : entry.selected && entry.adjustable === true
    ? `‹ ${entry.value} ›`
    : entry.value;
  const summary = [entry.hint, value]
    .filter((part): part is string => part !== undefined)
    .join(" · ");
  const rightColor = entry.selected ? pal.focus : pal.ink.muted;
  const right = summary === "" || (width <= 40 && entry.value === undefined)
    ? []
    : [{
        text: elide(
          summary,
          Math.max(1, Math.floor(entry.value === undefined ? width / 4 : width * 0.45)),
        ),
        fg: rightColor,
        bold: entry.selected || undefined,
      }];
  return row(width, primary, right);
}

function primaryWidth(entry: Pick<MenuEntry, "label">): number {
  return plainLen([{ text: entry.label }]);
}
