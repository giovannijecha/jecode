// Shared transcript inset and a bounded measure for model prose and tool records.

import type { Seg } from "../ui/render.ts";

const GUTTER = 2;
const MODEL_MEASURE = 86;

export function modelTranscriptWidth(width: number): number {
  return Math.max(8, Math.min(width - GUTTER * 2, MODEL_MEASURE));
}

export function insetTranscript(rows: readonly string[]): string[] {
  return rows.map((line) => line === "" ? "" : `${" ".repeat(GUTTER)}${line}`);
}

export function transcriptGutter(_width: number): number {
  return GUTTER;
}

export function transcriptWidth(width: number): number {
  return Math.max(8, width - transcriptGutter(width));
}

/** Put one semantic mark in the transcript margin, or reserve the same cells. */
export function transcriptLead(width: number, mark?: Seg): Seg[] {
  if (mark === undefined) return [{ text: " ".repeat(transcriptGutter(width)) }];
  return [
    ...transcriptMark(width, mark),
    { text: " " },
  ];
}

/** Draw only the gutter mark, without trailing content-column whitespace. */
export function transcriptMark(_width: number, mark: Seg): Seg[] {
  return [{ ...mark }];
}
