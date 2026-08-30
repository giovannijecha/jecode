// Typing one line into the dock, when what is being typed is not a message.
//
// The composer already knows how to be a line editor, so this borrows it whole
// — the same `Editor`, the same keymap — and adds the two things a composer
// must never do: hide what it holds, and stay on one row however long that is.

import type { Palette } from "../ui/theme.ts";
import type { Seg } from "../ui/render.ts";
import { row } from "../ui/render.ts";

import type { Editor } from "./editor.ts";
import { promptCursor, promptLine } from "./components/prompt.ts";

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

export function panel(field: Field, width: number, pal: Palette): string[] {
  const { ink } = pal;
  const head = row(
    width,
    field.title,
    field.right === undefined ? [] : [{ text: field.right, fg: ink.muted }],
  );

  const line = promptLine(field.editor.text, field.editor.cursor, width, pal, {
    secret: field.secret,
  }).row;

  const note =
    field.note === undefined ? [] : [row(width, [{ text: `  ${field.note}`, fg: ink.muted }])];

  return [head, line, ...note];
}

/** Where the caret sits, relative to the unframed field body. */
export function caret(field: Field, width: number): { row: number; col: number } {
  const at = promptCursor(field.editor.text, field.editor.cursor, width, {
    secret: field.secret,
  });
  return { row: 1, col: at.col };
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
