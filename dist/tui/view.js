// Compose one exact-height frame from the transcript and production chrome.
import { row } from "../ui/render.js";
import { elide } from "../ui/width.js";
import * as modal from "./modal.js";
import { renderCommandMenu } from "./components/command-menu.js";
import { renderComposer } from "./components/composer.js";
import { renderDock } from "./components/dock.js";
import { renderFooter } from "./components/footer.js";
import { renderStatus, spinner } from "./components/status.js";
import { transcriptRenderer } from "./transcript-view.js";
export { spinner } from "./components/status.js";
const DOCK_MAX = 6;
const FOOTER_ROWS = 1;
const DOCK_RULES = 2;
const TRANSCRIPT_DOCK_GAP = 1;
const TRANSCRIPT_MIN = 1;
const MIN_ROWS = 14;
const MIN_COLS = 38;
export function compose(view, size, transcript = transcriptRenderer()) {
    const height = Math.max(1, size.rows);
    const width = Math.max(1, size.cols);
    if (height < MIN_ROWS || width < MIN_COLS)
        return tooSmall(height, width, view);
    const dock = dockRows(view, width, height);
    const transcriptHeight = Math.max(1, height - dock.rows.length);
    // Any spare height belongs above the conversation, so short sessions grow
    // upward from the fixed dock rhythm instead of leaving a changing hole
    // beneath the latest reply.
    const viewport = transcript.viewport(view.blocks, width, transcriptHeight, view.scroll, view.pal);
    const cursor = dock.cursor === undefined
        ? undefined
        : { row: transcriptHeight + dock.cursor.row, col: dock.cursor.col };
    return { rows: [...viewport.rows, ...dock.rows], cursor, maxScroll: viewport.maxScroll };
}
function dockRows(view, width, height) {
    const status = renderStatus({
        // A modal already names the interaction in progress. Let command feedback
        // use the footer while that interaction is open instead of repeating a
        // generic "Running /…" label beside it.
        status: view.modal === undefined ? view.status : undefined,
        feedback: view.feedback,
        readiness: view.readiness,
        tick: view.spin,
        reducedMotion: view.reducedMotion === true,
        unseen: view.unseen ?? 0,
    }, view.pal);
    const footer = renderFooter(view.footer, status, width, view.pal);
    if (view.modal !== undefined) {
        const body = {
            rows: modal.panel(view.modal, width, view.pal, height - FOOTER_ROWS - TRANSCRIPT_MIN - TRANSCRIPT_DOCK_GAP - DOCK_RULES),
            cursor: modal.caret(view.modal, width),
        };
        const dock = renderDock(body, width, view.pal, true);
        return {
            rows: ["", ...dock.rows, ...footer],
            cursor: offset(dock.cursor, TRANSCRIPT_DOCK_GAP),
        };
    }
    const menu = renderCommandMenu(view.menu ?? [], view.menuIndex, width, view.pal);
    const available = height
        - TRANSCRIPT_MIN
        - TRANSCRIPT_DOCK_GAP
        - menu.length
        - FOOTER_ROWS
        - DOCK_RULES;
    const composer = renderComposer(view.editor, width, Math.max(1, Math.min(DOCK_MAX, available)), view.pal);
    const dock = renderDock({ rows: [...composer.rows, ...menu], cursor: composer.cursor }, width, view.pal, menu.length > 0);
    return {
        rows: ["", ...dock.rows, ...footer],
        cursor: offset(dock.cursor, TRANSCRIPT_DOCK_GAP),
    };
}
function offset(cursor, rows) {
    return cursor === undefined ? undefined : { row: cursor.row + rows, col: cursor.col };
}
function tooSmall(height, width, view) {
    const rows = Array.from({ length: height }, () => "");
    const middle = Math.floor(height / 2);
    rows[middle] = row(width, [
        { text: elide("jecode · terminal too small", Math.max(1, width)), fg: view.pal.accent, bold: true },
    ]);
    if (middle + 1 < height) {
        rows[middle + 1] = row(width, [
            { text: elide(`need ${MIN_COLS}×${MIN_ROWS}`, Math.max(1, width)), fg: view.pal.ink.muted },
        ]);
    }
    return { rows, maxScroll: 0 };
}
