import type { Palette } from "../../ui/theme.ts";
import { row } from "../../ui/render.ts";
import { charWidth, graphemes, textWidth } from "../../ui/width.ts";
import type { Editor } from "../editor.ts";
import type { Cursor } from "../frame.ts";

export type Composer = { rows: string[]; cursor: Cursor };

export function renderComposer(
  editor: Editor,
  width: number,
  maxInputRows: number,
  pal: Palette,
  right = "",
): Composer {
  const rightRoom = right === "" ? 0 : textWidth(right) + 1;
  const laid = layout(editor, Math.max(1, width - rightRoom));
  const room = Math.max(1, maxInputRows);
  const first = Math.max(0, Math.min(laid.lines.length - room, laid.cursor.row));
  const shown = laid.lines.slice(first, first + room);
  return {
    rows: shown.map((line, index) => row(
      width,
      [{ text: line, fg: pal.ink.bright }],
      index === 0 && right !== "" ? [{ text: right, fg: pal.ink.dim }] : [],
    )),
    cursor: { row: laid.cursor.row - first, col: laid.cursor.col },
  };
}

function layout(editor: Editor, width: number): { lines: string[]; cursor: Cursor } {
  const lines: string[] = [];
  let line = "";
  let used = 0;
  let index = 0;
  let cursor: Cursor = { row: 0, col: 0 };

  const place = (): void => {
    cursor = { row: lines.length, col: used };
  };
  const feed = (): void => {
    lines.push(line);
    line = "";
    used = 0;
  };

  for (const cluster of graphemes(editor.text)) {
    if (index === editor.cursor) place();
    index += cluster.length;
    if (cluster === "\n") {
      feed();
      continue;
    }
    const cells = charWidth(cluster);
    if (used + cells > width) feed();
    line += cluster;
    used += cells;
    if (used >= width) feed();
  }

  if (index === editor.cursor) place();
  lines.push(line);
  return { lines, cursor };
}
