import { blank, row } from "../../ui/render.js";
import { markdown } from "../../ui/markdown.js";
const PAD = 1;
export const REASONING_PREVIEW_ROWS = 3;
export function renderUser(block, width, pal) {
    const inner = Math.max(8, width - PAD * 2);
    const content = markdown(block.text, inner, pal, inner);
    return [
        "",
        blank(width, pal.surface.subtle),
        ...content.map((line) => row(width, line.segs, [], pal.surface.subtle, PAD)),
        blank(width, pal.surface.subtle),
    ];
}
export function renderAnswer(block, width, pal) {
    const inner = Math.max(8, width - PAD * 2);
    return [
        "",
        ...markdown(block.text, inner, pal, inner).map((line) => row(width, line.segs, [], undefined, PAD)),
    ];
}
export function renderReasoning(block, width, pal) {
    const inner = Math.max(8, width - PAD * 2);
    const content = markdown(block.text, inner, pal, inner);
    const expanded = block.expanded === true;
    const visible = expanded ? content : content.slice(-REASONING_PREVIEW_ROWS);
    const action = expanded
        ? "ctrl+o compact"
        : content.length > REASONING_PREVIEW_ROWS
            ? "ctrl+o full"
            : undefined;
    return [
        "",
        row(width, [
            {
                text: block.live === true ? "thinking" : "thought",
                fg: pal.ink.bright,
                bold: true,
            },
        ], action === undefined ? [] : [{ text: action, fg: pal.ink.muted }], undefined, PAD),
        ...visible.map((line) => row(width, line.segs.map((seg) => ({ ...seg, fg: pal.ink.muted, italic: true })), [], undefined, PAD)),
    ];
}
