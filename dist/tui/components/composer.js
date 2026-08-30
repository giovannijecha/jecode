import { row } from "../../ui/render.js";
import { charWidth, graphemes } from "../../ui/width.js";
export function renderComposer(editor, width, maxInputRows, pal) {
    const laid = layout(editor, Math.max(1, width));
    const room = Math.max(1, maxInputRows);
    const first = Math.max(0, Math.min(laid.lines.length - room, laid.cursor.row));
    const shown = laid.lines.slice(first, first + room);
    return {
        rows: shown.map((line) => row(width, [{ text: line, fg: pal.ink.bright }])),
        cursor: { row: laid.cursor.row - first, col: laid.cursor.col },
    };
}
function layout(editor, width) {
    const lines = [];
    let line = "";
    let used = 0;
    let index = 0;
    let cursor = { row: 0, col: 0 };
    const place = () => {
        cursor = { row: lines.length, col: used };
    };
    const feed = () => {
        lines.push(line);
        line = "";
        used = 0;
    };
    for (const cluster of graphemes(editor.text)) {
        if (index === editor.cursor)
            place();
        index += cluster.length;
        if (cluster === "\n") {
            feed();
            continue;
        }
        const cells = charWidth(cluster);
        if (used + cells > width)
            feed();
        line += cluster;
        used += cells;
        if (used >= width)
            feed();
    }
    if (index === editor.cursor)
        place();
    lines.push(line);
    return { lines, cursor };
}
