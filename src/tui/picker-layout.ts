// Bounded Ribbon composition; filtering and selection remain in picker.ts.

import type { Palette } from "../ui/theme.ts";
import { row } from "../ui/render.ts";
import type { Picker, Match } from "./picker.ts";
import type { DockBody } from "./components/dock.ts";
import { menuText, renderMenu } from "./components/menu.ts";
import { promptCursor, promptLine } from "./components/prompt.ts";

export function layoutPicker(
  picker: Picker, found: readonly Match[], width: number, pal: Palette | undefined, maxRows: number,
): DockBody {
  const budget = Math.max(0, maxRows);
  const queryRows = picker.searchable && budget >= 2 ? 1 : 0;
  const titleRows = picker.title.length > 0 && budget - queryRows >= 2 ? 1 : 0;
  const controlRows = budget - queryRows - titleRows >= 2 ? 1 : 0;
  let contextRoom = Math.max(0, budget - queryRows - titleRows - controlRows - 1);
  const target = menuText(picker.right ?? "", width, Math.min(2, contextRoom));
  contextRoom -= target.length;
  const note = menuText(picker.description ?? picker.footer ?? "", width, Math.min(2, contextRoom));
  const prefix = titleRows + target.length + note.length + queryRows;
  const menu = renderMenu(found.map(({ option, index }) => ({ ...option, selected: index === picker.index })),
    width, pal, {
      maxRows: budget - prefix - controlRows,
      visible: picker.visible ?? 6,
      overflow: picker.overflow,
    });
  const progress = found.length > menu.last - menu.first || picker.searchable
    ? `${found.length === 0 ? "0" : `${menu.first + 1}–${menu.last}`} / ${found.length}` +
      (found.length === picker.options.length ? "" : ` · ${picker.options.length} total`)
    : "";
  const query = picker.query ?? "";
  const cursor = queryRows === 0 ? undefined : {
    row: prefix - 1,
    col: promptCursor(query, query.length, width, { right: progress }).col,
  };
  if (pal === undefined) return { rows: [], cursor };
  const controls = picker.controls ?? (picker.adjust
    ? "↑↓ move · ←→ change · esc close" : "↑↓ move · enter select · esc close");
  return {
    rows: [
      ...(titleRows === 0 ? [] : [row(width, picker.title,
        queryRows > 0 || progress === "" ? [] : [{ text: progress, fg: pal.ink.dim }])]),
      ...[...target, ...note].map((text) => row(width, [{ text, fg: pal.ink.muted }])),
      ...(queryRows === 0 ? [] : [promptLine(query, query.length, width, pal, {
        placeholder: "type to filter", right: progress,
      }).row]),
      ...menu.rows,
      ...(controlRows === 0 ? [] : [row(width, [{ text: controls, fg: pal.ink.dim }])]),
    ], cursor,
  };
}
