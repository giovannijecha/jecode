// One active one-line input grammar for dock fields and searchable pickers.

import type { Palette } from "../../ui/theme.ts";
import type { Seg } from "../../ui/render.ts";
import { row } from "../../ui/render.ts";
import { charWidth, graphemes, textWidth } from "../../ui/width.ts";
import type { Cursor } from "../frame.ts";

const MARK = "→ ";
const DOT = "●";
export const PROMPT_WIDTH = textWidth(MARK);

export type PromptOptions = {
  placeholder?: string;
  right?: string;
  secret?: boolean;
};

export type PromptLine = { row: string; cursor: Cursor };

export function promptLine(
  text: string,
  cursor: number,
  width: number,
  pal: Palette,
  options: PromptOptions = {},
): PromptLine {
  const right: Seg[] = options.right === undefined || options.right === ""
    ? []
    : [{ text: options.right, fg: pal.ink.dim }];
  const laid = layout(text, cursor, width, options);
  const content: Seg = laid.visible === "" && options.placeholder !== undefined
    ? { text: options.placeholder, fg: pal.ink.dim }
    : { text: laid.visible, fg: pal.ink.bright };

  return {
    row: row(width, [{ text: MARK, fg: pal.accent }, content], right),
    cursor: { row: 0, col: PROMPT_WIDTH + textWidth(laid.before) },
  };
}

export function promptCursor(
  text: string,
  cursor: number,
  width: number,
  options: PromptOptions = {},
): Cursor {
  const laid = layout(text, cursor, width, options);
  return {
    row: 0,
    col: PROMPT_WIDTH + textWidth(laid.before),
  };
}

function layout(
  text: string,
  cursor: number,
  width: number,
  options: PromptOptions,
): { visible: string; before: string } {
  const rightRoom = options.right === undefined || options.right === ""
    ? 0
    : textWidth(options.right) + 1;
  const room = Math.max(1, width - PROMPT_WIDTH - rightRoom);
  const source = graphemes(text);
  const shown = options.secret === true ? source.map(() => DOT) : source;
  const at = graphemes(text.slice(0, cursor)).length;
  const start = visibleStart(shown, at, room);
  return {
    visible: visibleSlice(shown, start, room),
    before: shown.slice(start, Math.max(start, at)).join(""),
  };
}

/** Keep one terminal cell available for the caret at the end of the window. */
function visibleStart(clusters: readonly string[], cursor: number, room: number): number {
  const budget = Math.max(0, room - 1);
  let used = 0;
  let start = Math.max(0, Math.min(clusters.length, cursor));
  while (start > 0) {
    const width = charWidth(clusters[start - 1] as string);
    if (used + width > budget) break;
    start--;
    used += width;
  }
  return start;
}

function visibleSlice(clusters: readonly string[], start: number, room: number): string {
  let used = 0;
  let out = "";
  for (let index = start; index < clusters.length; index++) {
    const cluster = clusters[index] as string;
    const width = charWidth(cluster);
    if (used + width > room) break;
    out += cluster;
    used += width;
  }
  return out;
}
