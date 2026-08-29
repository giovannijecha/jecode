// Drawing the model's prose, which is Markdown whether or not we asked for it.
//
// Printing it raw is the single loudest way a terminal agent looks unfinished:
// the reader gets asterisks around the emphasis and backticks around the code,
// which is the notation, not the meaning. So the notation is consumed here and
// spent on weight and colour instead.
//
// Two rules keep this honest. It renders what a terminal can actually draw —
// emphasis is weight and colour, never a slant, because a slant many terminals
// drop leaves prose whose marks are already gone. And it is written to be fed
// a *prefix*: text arrives a token at a time, so an unterminated fence is a
// code block still being written, not a parse error.

import type { Palette, RGB } from "./theme.ts";
import type { Seg } from "./render.ts";
import { flow } from "./render.ts";
import type { Role } from "./highlight.ts";
import { highlight } from "./highlight.ts";
import { elide } from "./width.ts";
import { inline } from "./inline.ts";
import * as table from "./table.ts";

export type MdRow = { segs: Seg[] };

const FENCE = /^\s*(?:```|~~~)(.*)$/;
const HEADING = /^(#{1,6})\s+(.*)$/;
const BULLET = /^(\s*)[-*+]\s+(.*)$/;
const ORDERED = /^(\s*)(\d{1,3})[.)]\s+(.*)$/;
const QUOTE = /^\s*>\s?(.*)$/;
const HR = /^\s*(?:-{3,}|\*{3,}|_{3,})\s*$/;

export function markdown(text: string, max: number, pal: Palette, proseMax = max): MdRow[] {
  const { ink } = pal;
  const prose = Math.min(max, proseMax);
  const rows: MdRow[] = [];
  const lines = text.split("\n");

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i] as string;
    const fence = FENCE.exec(line);

    if (fence !== null) {
      i = code(lines, i, max, pal, rows);
      continue;
    }

    if (line.trim() === "") {
      rows.push({ segs: [] });
      continue;
    }

    if (table.opens(line, lines[i + 1])) {
      const read = table.parse(lines, i);
      rows.push(...table.draw(read.table, max, pal));
      i = read.end;
      continue;
    }

    if (HR.test(line)) {
      rows.push({ segs: [{ text: "─".repeat(Math.min(max, 80)), fg: pal.rule }] });
      continue;
    }

    const heading = HEADING.exec(line);
    if (heading !== null) {
      rows.push(...emit(inline(heading[2] as string, ink.bright, pal, true), prose, []));
      continue;
    }

    const quote = QUOTE.exec(line);
    if (quote !== null) {
      const bar: Seg[] = [{ text: "│ ", fg: pal.rule }];
      rows.push(...emit(inline(quote[1] as string, ink.muted, pal), prose - 2, bar, bar));
      continue;
    }

    const bullet = BULLET.exec(line);
    if (bullet !== null) {
      const depth = Math.min(2, Math.floor((bullet[1] as string).length / 2));
      const mark: Seg[] = [
        { text: "  ".repeat(depth) },
        { text: "- ", fg: pal.accent },
      ];
      const hang: Seg[] = [{ text: `${"  ".repeat(depth)}  ` }];
      rows.push(...emit(inline(bullet[2] as string, ink.fg, pal), prose - depth * 2 - 2, mark, hang));
      continue;
    }

    const ordered = ORDERED.exec(line);
    if (ordered !== null) {
      const label = `${ordered[2] as string}. `;
      const mark: Seg[] = [{ text: label, fg: pal.accent }];
      const hang: Seg[] = [{ text: " ".repeat(label.length) }];
      rows.push(...emit(inline(ordered[3] as string, ink.fg, pal), prose - label.length, mark, hang));
      continue;
    }

    // A paragraph is its lines joined, not one row each. The model hard-wraps
    // its prose at some width of its own, and honouring those breaks would
    // wrap already-wrapped text — the ragged half-rows that gives is the
    // clearest tell that nobody parsed the Markdown.
    const paragraph = [line];
    while (
      i + 1 < lines.length &&
      plain(lines[i + 1] as string) &&
      !table.opens(lines[i + 1] as string, lines[i + 2])
    ) {
      paragraph.push((lines[++i] as string).trim());
    }
    rows.push(...emit(inline(paragraph.join(" "), ink.fg, pal), prose, []));
  }

  return rows;
}

/** Whether a line continues a paragraph rather than starting something. */
function plain(line: string): boolean {
  if (line.trim() === "") return false;
  return (
    !FENCE.test(line) &&
    !HR.test(line) &&
    !HEADING.test(line) &&
    !QUOTE.test(line) &&
    !BULLET.test(line) &&
    !ORDERED.test(line)
  );
}

/**
 * A fenced block with muted transcript delimiters, never wrapped.
 *
 * Code that wraps stops being code — an indent that moved is a different
 * program. A line too wide keeps both ends and marks the middle instead.
 */
function code(lines: string[], start: number, max: number, pal: Palette, out: MdRow[]): number {
  const { ink } = pal;
  const open = FENCE.exec(lines[start] as string);
  const lang = (open?.[1] ?? "").trim();
  let i = start + 1;

  const body: string[] = [];
  for (; i < lines.length; i++) {
    const line = lines[i] as string;
    if (FENCE.test(line)) break;
    body.push(elide(line.replace(/\t/g, "  "), max - 2));
  }

  out.push({ segs: [{ text: `\`\`\`${lang}`, fg: ink.muted }] });

  const roles: Record<Role, RGB> = {
    plain: ink.added,
    comment: ink.muted,
    string: ink.added,
    number: ink.attention,
    keyword: pal.accent,
  };

  for (const tokens of highlight(body, lang)) {
    out.push({
      segs: [
        { text: "  " },
        ...tokens.map((token) => ({ text: token.text, fg: roles[token.role] })),
      ],
    });
  }

  out.push({ segs: [{ text: "```", fg: ink.muted }] });
  return i;
}

/** Flow one inline run into rows, with an opener and a hanging indent. */
function emit(segs: Seg[], max: number, opener: Seg[], hang: Seg[] = []): MdRow[] {
  return flow(segs, Math.max(8, max), hang).map((line, index) => ({
    segs: index === 0 ? [...opener, ...line] : line,
  }));
}
