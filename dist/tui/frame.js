// Painting a frame by writing only the rows that changed.
//
// The whole frame is recomposed on every render — that is what makes the view
// a pure function of the state. What must not happen on every render is
// writing it: a full repaint at streaming speed flickers and floods the pipe.
// So the painter keeps the last frame and emits the difference.
import { CSI } from "../ui/render.js";
import { CURSOR, SYNC, write } from "./screen.js";
export function painter() {
    let previous = [];
    return {
        paint(rows, cursor) {
            let out = SYNC.begin + CURSOR.hide;
            const height = Math.max(rows.length, previous.length);
            for (let i = 0; i < height; i++) {
                const next = rows[i] ?? "";
                if (next === previous[i])
                    continue;
                out += `${CSI}${i + 1};1H${CSI}2K${next}`;
            }
            if (cursor !== undefined) {
                out += `${CSI}${cursor.row + 1};${cursor.col + 1}H${CURSOR.show}`;
            }
            previous = rows.slice();
            // One write, so a terminal that ignores the synchronization hint still
            // gets the frame as a single arrival rather than a row at a time.
            write(out + SYNC.end);
        },
        invalidate() {
            previous = [];
        },
    };
}
