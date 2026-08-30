import { row, wrap } from "../../ui/render.js";
export function renderNotice(block, width, pal) {
    const fg = {
        info: pal.ink.muted,
        warn: pal.ink.attention,
        error: pal.ink.removed,
    };
    const mark = block.tone === "error" ? "× " : block.tone === "warn" ? "! " : "· ";
    return [
        "",
        ...wrap(block.text, Math.max(1, width - 3)).map((line, index) => row(width, [
            { text: index === 0 ? mark : "  ", fg: fg[block.tone], bold: index === 0 },
            { text: line, fg: fg[block.tone] },
        ], [], undefined, 1)),
    ];
}
export function renderList(block, width, pal) {
    return [
        "",
        ...block.items.map((item) => row(width, [{ text: item.text, fg: item.dim ? pal.ink.muted : pal.ink.fg }], [], undefined, 1)),
    ];
}
