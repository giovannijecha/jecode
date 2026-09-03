// One semantic margin shared by every transcript block.

import type { Seg } from "../ui/render.ts";

const FULL_GUTTER = 4;
const COMPACT_GUTTER = 2;
const COMPACT_BELOW = 64;

export function transcriptGutter(width: number): number {
  return width < COMPACT_BELOW ? COMPACT_GUTTER : FULL_GUTTER;
}

export function transcriptWidth(width: number): number {
  return Math.max(8, width - transcriptGutter(width));
}

/** Put one semantic mark in the transcript margin, or reserve the same cells. */
export function transcriptLead(width: number, mark?: Seg): Seg[] {
  if (mark === undefined) return [{ text: " ".repeat(transcriptGutter(width)) }];
  return [
    ...transcriptMark(width, mark),
    { text: " ".repeat(width < COMPACT_BELOW ? 1 : 2) },
  ];
}

/** Draw only the gutter mark, without trailing content-column whitespace. */
export function transcriptMark(width: number, mark: Seg): Seg[] {
  if (width < COMPACT_BELOW) return [{ ...mark }];
  return [{ text: " " }, { ...mark }];
}
