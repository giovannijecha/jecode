// Markdown tables, drawn as a table rather than printed as pipes.
//
// A table is the one Markdown construct whose meaning is its geometry: the
// columns *are* the information. Left as source it is the worst thing on the
// screen — a wall of `|` where the eye has to do the aligning by hand.
//
// Two decisions shape the drawing. Columns are measured in cells, never in
// characters, so a table of Japanese or emoji lines up like a table of ASCII.
// And there are no vertical rules: a column of `│` down every gap is four more
// glyphs per row earning nothing that whitespace and one horizontal rule under
// the header do not already earn.

import type { Palette, RGB } from "./theme.ts";
import type { Seg } from "./render.ts";
import { fitSegs, plainLen } from "./render.ts";
import { inline } from "./inline.ts";

export type Align = "left" | "right" | "center";
export type Table = { head: string[]; body: string[][]; align: Align[] };

/** A drawn row: the same shape `markdown` emits, so it can be spliced in. */
type Line = { segs: Seg[] };

/** Cells between the gaps. Two spaces reads as a column break; one does not. */
const GAP = 2;
/** Narrowest a column may be squeezed to before the text stops being readable. */
const MIN = 4;

const DELIMITER = /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/;

/**
 * Whether a line opens a table.
 *
 * A table is only a table once its delimiter has arrived — which matters,
 * because text streams in. A header row on its own is a paragraph that happens
 * to contain pipes, and it is drawn as one until the next line proves otherwise.
 */
export function opens(line: string, next: string | undefined): boolean {
  return line.includes("|") && next !== undefined && DELIMITER.test(next);
}

/** Read the table starting at `start`, and say which line it ended on. */
export function parse(lines: readonly string[], start: number): { table: Table; end: number } {
  const head = cells(lines[start] as string);
  const align = (lines[start + 1] as string)
    .split("|")
    .map((cell) => cell.trim())
    .filter((cell) => cell !== "")
    .map(alignOf);

  const body: string[][] = [];
  let i = start + 2;
  for (; i < lines.length; i++) {
    const line = lines[i] as string;
    if (!line.includes("|") || line.trim() === "") break;
    body.push(cells(line));
  }

  return { table: { head, body, align }, end: i - 1 };
}

function cells(line: string): string[] {
  const parts = line.split("|").map((cell) => cell.trim());
  // A row may or may not be fenced by outer pipes; both spellings are legal,
  // and the difference is two empty cells nobody wrote.
  if (parts[0] === "") parts.shift();
  if (parts.length > 0 && parts[parts.length - 1] === "") parts.pop();
  return parts;
}

function alignOf(spec: string): Align {
  const left = spec.startsWith(":");
  const right = spec.endsWith(":");
  if (left && right) return "center";
  return right ? "right" : "left";
}

export function draw(table: Table, max: number, pal: Palette): Line[] {
  const { ink } = pal;
  const count = Math.max(table.head.length, ...table.body.map((r) => r.length), 1);
  const widths = fit(natural(table, count, pal), max);

  const head: Line = {
    segs: laid(table.head, widths, table, pal, ink.bright, true),
  };

  const under: Line = {
    segs: [{ text: widths.map((w) => "─".repeat(w)).join(" ".repeat(GAP)), fg: pal.rule }],
  };

  const body = table.body.map((cells) => ({
    segs: laid(cells, widths, table, pal, ink.fg, false),
  }));

  return [head, under, ...body];
}

/** How wide each column would like to be: its widest cell. */
function natural(table: Table, count: number, pal: Palette): number[] {
  const widths = Array.from({ length: count }, () => 0);

  for (const cells of [table.head, ...table.body]) {
    cells.forEach((cell, i) => {
      if (i < count) widths[i] = Math.max(widths[i] as number, plainLen(inline(cell, pal.ink.fg, pal)));
    });
  }
  return widths;
}

/**
 * Squeeze the columns into the room there is, widest first.
 *
 * Taking a cell off the widest column each pass is what keeps a table of one
 * long prose column and three short numeric ones readable: the prose gives way,
 * and the numbers — which cannot be elided without becoming wrong — do not.
 */
function fit(widths: number[], max: number): number[] {
  const out = [...widths];
  const gaps = GAP * (out.length - 1);

  let total = out.reduce((n, w) => n + w, 0) + gaps;
  while (total > max) {
    const widest = out.reduce((best, w, i) => (w > (out[best] as number) ? i : best), 0);
    if ((out[widest] as number) <= MIN) break;
    out[widest] = (out[widest] as number) - 1;
    total--;
  }

  return out;
}

/** One row's cells clipped to their columns and padded to their alignment. */
function laid(
  cells: readonly string[],
  widths: readonly number[],
  table: Table,
  pal: Palette,
  base: RGB,
  bold: boolean,
): Seg[] {
  const segs: Seg[] = [];

  widths.forEach((width, i) => {
    if (i > 0) segs.push({ text: " ".repeat(GAP) });

    const content = fitSegs(inline(cells[i] ?? "", base, pal, bold), width);
    const slack = Math.max(0, width - plainLen(content));
    const align = table.align[i] ?? "left";
    const before = align === "right" ? slack : align === "center" ? Math.floor(slack / 2) : 0;

    if (before > 0) segs.push({ text: " ".repeat(before) });
    segs.push(...content);
    if (slack - before > 0) segs.push({ text: " ".repeat(slack - before) });
  });

  return segs;
}
