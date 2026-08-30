// Which key does which edit.
//
// Pure transforms over the input line, kept apart from the shell so that the
// keymap can be read — and changed — without reading the turn machinery.
import * as edit from "./editor.js";
import { terminalText } from "../ui/terminal-text.js";
/** The new line, or undefined when the key is not an editing key. */
export function applyKey(state, key) {
    switch (key.name) {
        case "char":
        case "paste":
            // A pasted CR is a line break, not a submit: the shell only submits on a
            // keypress it saw as enter.
            return edit.insert(state, terminalText(key.text.replace(/\r\n?/g, "\n"), { multiline: true }));
        case "newline":
            return edit.insert(state, "\n");
        case "backspace":
            return edit.backspace(state);
        case "delete":
            return edit.del(state);
        case "left":
            return edit.left(state);
        case "right":
            return edit.right(state);
        case "wordleft":
            return edit.wordLeft(state);
        case "wordright":
            return edit.wordRight(state);
        case "home":
            return edit.home(state);
        case "end":
            return edit.end(state);
        default:
            break;
    }
    if (!key.ctrl)
        return undefined;
    switch (key.name) {
        case "a":
            return edit.home(state);
        case "e":
            return edit.end(state);
        case "u":
            return edit.killToStart(state);
        case "k":
            return edit.killToEnd(state);
        case "w":
            return edit.killWord(state);
        default:
            return undefined;
    }
}
