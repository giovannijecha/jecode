// Typing one line into the dock, when what is being typed is not a message.
//
// The composer already knows how to be a line editor, so this borrows it whole
// — the same `Editor`, the same keymap — and adds the two things a composer
// must never do: hide what it holds, and stay on one row however long that is.

import type { Palette } from "../ui/theme.ts";
import type { Seg } from "../ui/render.ts";
import { row } from "../ui/render.ts";
import { graphemes, textWidth } from "../ui/width.ts";

import type { Editor } from "./editor.ts";

export type Field = {
  /** The question, already styled. */
  title: Seg[];
  /** Shown at the right of the title row. */
  right?: string;
  editor: Editor;
  /**
   * Draw a dot per character instead of the character.
   *
   * Not a display preference. A key on screen is a key in a screen recording,
   * in a screenshot pasted into an issue, and over the shoulder of whoever is
   * behind you — and unlike a password it is usually pasted, so it arrives all
   * at once and stays there until the field closes.
   */
  secret: boolean;
  /** A line under the input: what is expected, or what went wrong. */
  note?: string;
};

const DOT = "●";
const PROMPT = "→ ";
const LEAD = textWidth(PROMPT);

export function panel(field: Field, width: number, pal: Palette): string[] {
  const { ink } = pal;
  const head = row(
    width,
    field.title,
    field.right === undefined ? [] : [{ text: field.right, fg: ink.muted }],
  );

  const view = laid(field, width);
  const line = row(
    width,
    [
      { text: PROMPT, fg: pal.accent },
      { text: view.text, fg: ink.bright },
    ],
  );

  const note =
    field.note === undefined ? [] : [row(width, [{ text: `  ${field.note}`, fg: ink.muted }])];

  return [head, line, ...note];
}

/** Where the caret sits, relative to the unframed field body. */
export function caret(field: Field, width: number): { row: number; col: number } {
  return { row: 1, col: LEAD + laid(field, width).col };
}

/**
 * The visible slice of the line, and where the caret lands inside it.
 *
 * A key is longer than most terminals are wide, so the line scrolls under a
 * fixed window rather than wrapping: an input that grows downwards pushes the
 * rest of the dock around while it is being pasted into.
 */
function laid(field: Field, width: number): { text: string; col: number } {
  const inner = Math.max(1, width - LEAD);
  const all = field.secret ? DOT.repeat(size(field.editor.text)) : field.editor.text;
  const at = size(field.editor.text.slice(0, field.editor.cursor));

  // Keep the caret in view, and prefer showing the end of what was typed —
  // which for a pasted key is the half that says the paste arrived whole.
  const start = Math.max(0, at - inner + 1);
  return { text: [...all].slice(start, start + inner).join(""), col: at - start };
}

function size(text: string): number {
  let n = 0;
  for (const _cluster of graphemes(text)) n++;
  return n;
}

/**
 * The line with its breaks taken out.
 *
 * A key is one line. A pasted one usually carries the newline that ended it,
 * and a field that grows a second row on paste is a field that has already
 * lost its layout — the break is the end of the paste, never content.
 */
export function oneLine(editor: Editor): Editor {
  if (!editor.text.includes("\n")) return editor;
  const kept = editor.text.slice(0, editor.cursor).replace(/\n/g, "");
  return { text: editor.text.replace(/\n/g, ""), cursor: kept.length };
}
