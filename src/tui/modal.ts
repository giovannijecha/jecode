// What can take the dock over, and how it draws.
//
// Two kinds, and the union exists so that everything between the command that
// opens one and the frame that draws it — the view, the shell, the key handler
// — speaks about "the thing that is open" rather than about a picker and a
// field separately. A third kind would be added here and nowhere else.

import type { Palette } from "../ui/theme.ts";
import type { Cursor } from "./frame.ts";
import type { Picker } from "./picker.ts";
import * as picker from "./picker.ts";
import type { Field } from "./field.ts";
import * as field from "./field.ts";

export type Modal = { kind: "pick"; picker: Picker } | { kind: "type"; field: Field };

export function panel(modal: Modal, width: number, pal: Palette, maxRows?: number): string[] {
  return modal.kind === "pick"
    ? picker.panel(modal.picker, width, pal, maxRows)
    : maxRows === undefined
      ? field.panel(modal.field, width, pal)
      : field.panel(modal.field, width, pal).slice(0, maxRows);
}

/**
 * Where the caret goes while this is open, if it goes anywhere.
 *
 * A menu is read, not typed into, so it hides the caret entirely — a blinking
 * block on a row nobody is editing is an invitation to type into it.
 */
export function caret(modal: Modal, width: number): Cursor | undefined {
  return modal.kind === "type" ? field.caret(modal.field, width) : undefined;
}
