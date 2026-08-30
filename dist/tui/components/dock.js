// The one shell shared by every interaction at the bottom of the screen.
import { rule } from "../../ui/render.js";
export function renderDock(body, width, pal, active) {
    return {
        rows: [rule(width, active ? pal.focus : pal.rule), ...body.rows, rule(width, active ? pal.focus : pal.rule)],
        cursor: body.cursor === undefined
            ? undefined
            : { row: body.cursor.row + 1, col: body.cursor.col },
    };
}
