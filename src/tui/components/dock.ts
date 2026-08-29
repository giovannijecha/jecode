// The one shell shared by every interaction at the bottom of the screen.

import type { Palette } from "../../ui/theme.ts";
import { rule } from "../../ui/render.ts";
import type { Cursor } from "../frame.ts";

export type DockBody = { rows: string[]; cursor?: Cursor };

export function renderDock(
  body: DockBody,
  width: number,
  pal: Palette,
  active: boolean,
): DockBody {
  return {
    rows: [rule(width, active ? pal.focus : pal.rule), ...body.rows, rule(width, active ? pal.focus : pal.rule)],
    cursor: body.cursor === undefined
      ? undefined
      : { row: body.cursor.row + 1, col: body.cursor.col },
  };
}
