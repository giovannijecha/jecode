import type { Palette } from "../../ui/theme.ts";
import type { Seg } from "../../ui/render.ts";
import { fitSegs, plainLen, row } from "../../ui/render.ts";
import { elide, textWidth } from "../../ui/width.ts";

export type FooterInfo = {
  workspace: string;
  model: string;
  effort: string;
};

export function renderFooter(
  info: FooterInfo,
  status: readonly Seg[],
  width: number,
  pal: Palette,
): string[] {
  const rightLimit = Math.min(width, Math.max(12, Math.floor(width * 0.58)));
  const right = fitSegs(status, rightLimit);
  const leftRoom = Math.max(0, width - plainLen(right) - (right.length === 0 ? 0 : 1));
  const left = identity(info, leftRoom);

  return [row(width, left === "" ? [] : [{ text: left, fg: pal.ink.dim }], right)];
}

function identity(info: FooterInfo, cols: number): string {
  if (cols <= 0) return "";
  const core = `${info.model || "no model"} · ${info.effort}`;
  if (textWidth(core) >= cols || info.workspace === "") return elide(core, cols);

  const divider = " · ";
  const workspaceRoom = cols - textWidth(core) - textWidth(divider);
  return workspaceRoom <= 0
    ? core
    : `${core}${divider}${elide(info.workspace, workspaceRoom)}`;
}
