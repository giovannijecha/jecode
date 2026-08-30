// What can take the dock over, and how it draws.
//
// Two kinds, and the union exists so that everything between the command that
// opens one and the frame that draws it — the view, the shell, the key handler
// — speaks about "the thing that is open" rather than about a picker and a
// field separately. A third kind would be added here and nowhere else.
import * as picker from "./picker.js";
import * as field from "./field.js";
export function panel(modal, width, pal, maxRows) {
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
export function caret(modal, width) {
    return modal.kind === "type" ? field.caret(modal.field, width) : undefined;
}
