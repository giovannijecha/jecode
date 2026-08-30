// Typing one line into the dock, when what is being typed is not a message.
//
// The composer already knows how to be a line editor, so this borrows it whole
// — the same `Editor`, the same keymap — and adds the two things a composer
// must never do: hide what it holds, and stay on one row however long that is.
import { row } from "../ui/render.js";
import { graphemes, textWidth } from "../ui/width.js";
const DOT = "●";
const PROMPT = "→ ";
const LEAD = textWidth(PROMPT);
export function panel(field, width, pal) {
    const { ink } = pal;
    const head = row(width, field.title, field.right === undefined ? [] : [{ text: field.right, fg: ink.muted }]);
    const view = laid(field, width);
    const line = row(width, [
        { text: PROMPT, fg: pal.accent },
        { text: view.text, fg: ink.bright },
    ]);
    const note = field.note === undefined ? [] : [row(width, [{ text: `  ${field.note}`, fg: ink.muted }])];
    return [head, line, ...note];
}
/** Where the caret sits, relative to the unframed field body. */
export function caret(field, width) {
    return { row: 1, col: LEAD + laid(field, width).col };
}
/**
 * The visible slice of the line, and where the caret lands inside it.
 *
 * A key is longer than most terminals are wide, so the line scrolls under a
 * fixed window rather than wrapping: an input that grows downwards pushes the
 * rest of the dock around while it is being pasted into.
 */
function laid(field, width) {
    const inner = Math.max(1, width - LEAD);
    const all = field.secret ? DOT.repeat(size(field.editor.text)) : field.editor.text;
    const at = size(field.editor.text.slice(0, field.editor.cursor));
    // Keep the caret in view, and prefer showing the end of what was typed —
    // which for a pasted key is the half that says the paste arrived whole.
    const start = Math.max(0, at - inner + 1);
    return { text: [...all].slice(start, start + inner).join(""), col: at - start };
}
function size(text) {
    let n = 0;
    for (const _cluster of graphemes(text))
        n++;
    return n;
}
/**
 * The line with its breaks taken out.
 *
 * A key is one line. A pasted one usually carries the newline that ended it,
 * and a field that grows a second row on paste is a field that has already
 * lost its layout — the break is the end of the paste, never content.
 */
export function oneLine(editor) {
    if (!editor.text.includes("\n"))
        return editor;
    const kept = editor.text.slice(0, editor.cursor).replace(/\n/g, "");
    return { text: editor.text.replace(/\n/g, ""), cursor: kept.length };
}
