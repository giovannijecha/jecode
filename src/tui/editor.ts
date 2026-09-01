// The input line: text and a cursor, plus the edits a terminal user expects.
//
// Pure functions over an immutable state — no rendering, no I/O — so the whole
// line editor is testable without a terminal.

import { graphemes } from "../ui/width.ts";

export type Editor = { readonly text: string; readonly cursor: number };

export const EMPTY: Editor = { text: "", cursor: 0 };

export function of(text: string): Editor {
  return { text, cursor: text.length };
}

export function insert(state: Editor, chunk: string): Editor {
  return {
    text: state.text.slice(0, state.cursor) + chunk + state.text.slice(state.cursor),
    cursor: state.cursor + chunk.length,
  };
}

export function backspace(state: Editor): Editor {
  if (state.cursor === 0) return state;
  const from = before(state.text, state.cursor);
  return { text: state.text.slice(0, from) + state.text.slice(state.cursor), cursor: from };
}

export function del(state: Editor): Editor {
  if (state.cursor >= state.text.length) return state;
  const to = after(state.text, state.cursor);
  return { text: state.text.slice(0, state.cursor) + state.text.slice(to), cursor: state.cursor };
}

export function left(state: Editor): Editor {
  return state.cursor === 0 ? state : { ...state, cursor: before(state.text, state.cursor) };
}

export function right(state: Editor): Editor {
  return state.cursor >= state.text.length
    ? state
    : { ...state, cursor: after(state.text, state.cursor) };
}

export function home(state: Editor): Editor {
  return { ...state, cursor: 0 };
}

export function end(state: Editor): Editor {
  return { ...state, cursor: state.text.length };
}

export function wordLeft(state: Editor): Editor {
  return { ...state, cursor: startOfWordBefore(state.text, state.cursor) };
}

export function wordRight(state: Editor): Editor {
  return { ...state, cursor: startOfWordAfter(state.text, state.cursor) };
}

/** ctrl+w — delete the word behind the cursor. */
export function killWord(state: Editor): Editor {
  const from = startOfDeletionBefore(state.text, state.cursor);
  if (from === state.cursor) return state;
  return { text: state.text.slice(0, from) + state.text.slice(state.cursor), cursor: from };
}

/** ctrl+delete — delete the word ahead of the cursor. */
export function killNextWord(state: Editor): Editor {
  const to = endOfDeletionAfter(state.text, state.cursor);
  if (to === state.cursor) return state;
  return {
    text: state.text.slice(0, state.cursor) + state.text.slice(to),
    cursor: state.cursor,
  };
}

/** ctrl+u — delete everything behind the cursor. */
export function killToStart(state: Editor): Editor {
  return { text: state.text.slice(state.cursor), cursor: 0 };
}

/** ctrl+k — delete everything ahead of the cursor. */
export function killToEnd(state: Editor): Editor {
  return { text: state.text.slice(0, state.cursor), cursor: state.cursor };
}

/**
 * Cursor motion is by grapheme, never by code unit.
 *
 * An emoji is two code units and an accented letter can be two more; stepping
 * over half of either does not move the cursor one place to the left, it
 * breaks the string — and a lone surrogate is what the terminal then draws.
 */
function before(text: string, cursor: number): number {
  let at = 0;
  for (const cluster of graphemes(text)) {
    const next = at + cluster.length;
    if (next >= cursor) return at;
    at = next;
  }
  return at;
}

function after(text: string, cursor: number): number {
  let at = 0;
  for (const cluster of graphemes(text)) {
    const next = at + cluster.length;
    if (at >= cursor) return next;
    at = next;
  }
  return text.length;
}

function startOfWordBefore(text: string, cursor: number): number {
  let i = cursor;
  while (i > 0 && whitespace(text[i - 1])) i--;
  while (i > 0 && !whitespace(text[i - 1])) i--;
  return i;
}

function startOfWordAfter(text: string, cursor: number): number {
  let i = cursor;
  while (i < text.length && !whitespace(text[i])) i++;
  while (i < text.length && whitespace(text[i])) i++;
  return i;
}

function startOfDeletionBefore(text: string, cursor: number): number {
  if (text[cursor - 1] === "\n") return cursor - 1;
  let i = cursor;
  while (i > 0 && horizontalWhitespace(text[i - 1])) i--;
  if (text[i - 1] === "\n") return i;
  while (i > 0 && !whitespace(text[i - 1])) i--;
  return i;
}

function endOfDeletionAfter(text: string, cursor: number): number {
  if (text[cursor] === "\n") return cursor + 1;
  let i = cursor;
  if (horizontalWhitespace(text[i])) {
    while (i < text.length && horizontalWhitespace(text[i])) i++;
    if (text[i] === "\n") return i;
    while (i < text.length && !whitespace(text[i])) i++;
    return i;
  }

  while (i < text.length && !whitespace(text[i])) i++;
  while (i < text.length && horizontalWhitespace(text[i])) i++;
  return i;
}

function horizontalWhitespace(value: string | undefined): boolean {
  return value !== undefined && value !== "\n" && /\s/u.test(value);
}

function whitespace(value: string | undefined): boolean {
  return value !== undefined && /\s/u.test(value);
}
