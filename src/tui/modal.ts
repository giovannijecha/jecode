// What can take the dock over, and how it draws.
//
// Three kinds, and the union exists so that everything between the command that
// opens one and the frame that draws it — the view, the shell, the key handler
// — speaks about "the thing that is open" rather than about each interaction
// separately.

import type { Palette } from "../ui/theme.ts";
import type { Cursor } from "./frame.ts";
import type { Picker } from "./picker.ts";
import * as picker from "./picker.ts";
import type { Field } from "./field.ts";
import * as field from "./field.ts";
import * as help from "./help.ts";

export type Modal =
  | { kind: "pick"; picker: Picker }
  | { kind: "type"; field: Field }
  | { kind: "help" };

export function panel(modal: Modal, width: number, pal: Palette, maxRows?: number): string[] {
  switch (modal.kind) {
    case "pick":
      return picker.panel(modal.picker, width, pal, maxRows);
    case "type":
      return maxRows === undefined
        ? field.panel(modal.field, width, pal)
        : field.panel(modal.field, width, pal).slice(0, maxRows);
    case "help":
      return help.panel(width, pal, maxRows);
  }
}

/**
 * Where the caret goes while this is open, if it goes anywhere.
 *
 * Menus without search hide the caret; searchable menus expose the shared
 * query prompt and place the terminal caret at its real editing position.
 */
export function caret(modal: Modal, width: number, maxRows?: number): Cursor | undefined {
  switch (modal.kind) {
    case "pick":
      return picker.caret(modal.picker, width, maxRows);
    case "type":
      return field.caret(modal.field, width);
    case "help":
      return undefined;
  }
}
