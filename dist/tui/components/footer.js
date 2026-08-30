import { fitSegs, plainLen, row } from "../../ui/render.js";
import { elide, textWidth } from "../../ui/width.js";
export function renderFooter(info, status, width, pal) {
    const rightLimit = Math.min(width, Math.max(12, Math.floor(width * 0.58)));
    const right = fitSegs(status, rightLimit);
    const leftRoom = Math.max(0, width - plainLen(right) - (right.length === 0 ? 0 : 1));
    const left = identity(info, leftRoom);
    return [row(width, left === "" ? [] : [{ text: left, fg: pal.ink.muted }], right)];
}
function identity(info, cols) {
    if (cols <= 0)
        return "";
    const core = `${info.model || "no model"} · ${info.effort}`;
    if (textWidth(core) >= cols || info.workspace === "")
        return elide(core, cols);
    const divider = " · ";
    const workspaceRoom = cols - textWidth(core) - textWidth(divider);
    return workspaceRoom <= 0
        ? core
        : `${core}${divider}${elide(info.workspace, workspaceRoom)}`;
}
