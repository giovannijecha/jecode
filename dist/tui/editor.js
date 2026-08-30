// The input line: text and a cursor, plus the edits a terminal user expects.
//
// Pure functions over an immutable state — no rendering, no I/O — so the whole
// line editor is testable without a terminal.
import { graphemes } from "../ui/width.js";
export const EMPTY = { text: "", cursor: 0 };
export function of(text) {
    return { text, cursor: text.length };
}
export function insert(state, chunk) {
    return {
        text: state.text.slice(0, state.cursor) + chunk + state.text.slice(state.cursor),
        cursor: state.cursor + chunk.length,
    };
}
export function backspace(state) {
    if (state.cursor === 0)
        return state;
    const from = before(state.text, state.cursor);
    return { text: state.text.slice(0, from) + state.text.slice(state.cursor), cursor: from };
}
export function del(state) {
    if (state.cursor >= state.text.length)
        return state;
    const to = after(state.text, state.cursor);
    return { text: state.text.slice(0, state.cursor) + state.text.slice(to), cursor: state.cursor };
}
export function left(state) {
    return state.cursor === 0 ? state : { ...state, cursor: before(state.text, state.cursor) };
}
export function right(state) {
    return state.cursor >= state.text.length
        ? state
        : { ...state, cursor: after(state.text, state.cursor) };
}
export function home(state) {
    return { ...state, cursor: 0 };
}
export function end(state) {
    return { ...state, cursor: state.text.length };
}
export function wordLeft(state) {
    return { ...state, cursor: startOfWordBefore(state.text, state.cursor) };
}
export function wordRight(state) {
    return { ...state, cursor: endOfWordAfter(state.text, state.cursor) };
}
/** ctrl+w — delete the word behind the cursor. */
export function killWord(state) {
    const from = startOfWordBefore(state.text, state.cursor);
    if (from === state.cursor)
        return state;
    return { text: state.text.slice(0, from) + state.text.slice(state.cursor), cursor: from };
}
/** ctrl+u — delete everything behind the cursor. */
export function killToStart(state) {
    return { text: state.text.slice(state.cursor), cursor: 0 };
}
/** ctrl+k — delete everything ahead of the cursor. */
export function killToEnd(state) {
    return { text: state.text.slice(0, state.cursor), cursor: state.cursor };
}
/**
 * Cursor motion is by grapheme, never by code unit.
 *
 * An emoji is two code units and an accented letter can be two more; stepping
 * over half of either does not move the cursor one place to the left, it
 * breaks the string — and a lone surrogate is what the terminal then draws.
 */
function before(text, cursor) {
    let at = 0;
    for (const cluster of graphemes(text)) {
        const next = at + cluster.length;
        if (next >= cursor)
            return at;
        at = next;
    }
    return at;
}
function after(text, cursor) {
    let at = 0;
    for (const cluster of graphemes(text)) {
        const next = at + cluster.length;
        if (at >= cursor)
            return next;
        at = next;
    }
    return text.length;
}
function startOfWordBefore(text, cursor) {
    let i = cursor;
    while (i > 0 && text[i - 1] === " ")
        i--;
    while (i > 0 && text[i - 1] !== " ")
        i--;
    return i;
}
function endOfWordAfter(text, cursor) {
    let i = cursor;
    while (i < text.length && text[i] === " ")
        i++;
    while (i < text.length && text[i] !== " ")
        i++;
    return i;
}
