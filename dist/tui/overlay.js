// The modal interaction layer: one picker or one single-line field.
import * as picker from "./picker.js";
import { oneLine } from "./field.js";
import { applyKey } from "./input.js";
export function shown(open) {
    if (open === undefined)
        return undefined;
    return "picker" in open ? { kind: "pick", picker: open.picker } : { kind: "type", field: open.field };
}
export function cancel(open) {
    if (open === undefined)
        return undefined;
    open.settle(undefined);
    return undefined;
}
export function handle(open, key) {
    if (key.ctrl && key.name === "c") {
        cancel(open);
        return { abort: true };
    }
    if (key.ctrl && key.name === "d") {
        cancel(open);
        return { quit: true };
    }
    if (key.name === "escape") {
        cancel(open);
        return {};
    }
    return "picker" in open ? handlePicker(open, key) : handleField(open, key);
}
function handlePicker(open, key) {
    switch (key.name) {
        case "up":
            open.picker = picker.move(open.picker, -1);
            break;
        case "down":
            open.picker = picker.move(open.picker, 1);
            break;
        case "home":
            open.picker = picker.edge(open.picker, "home");
            break;
        case "end":
            open.picker = picker.edge(open.picker, "end");
            break;
        case "pageup":
            open.picker = picker.page(open.picker, -1);
            break;
        case "pagedown":
            open.picker = picker.page(open.picker, 1);
            break;
        case "backspace":
            open.picker = picker.backspace(open.picker);
            break;
        case "enter":
            if (picker.selected(open.picker) === undefined)
                break;
            open.settle(open.picker.index);
            return {};
        case "paste":
            if (open.picker.searchable === true) {
                open.picker = picker.type(open.picker, key.text.replace(/\s+/g, " "));
            }
            break;
        case "char":
            if (open.picker.searchable === true) {
                open.picker = picker.type(open.picker, key.text);
                break;
            }
            {
                const index = picker.byKey(open.picker, key.text);
                if (index === undefined)
                    break;
                open.settle(index);
                return {};
            }
        default:
            if (key.ctrl && key.name === "u" && open.picker.searchable === true) {
                open.picker = picker.clear(open.picker);
            }
    }
    return { open };
}
function handleField(open, key) {
    if (key.name === "enter") {
        const text = open.field.editor.text.trim();
        open.settle(text === "" ? undefined : text);
        return {};
    }
    const edited = applyKey(open.field.editor, key);
    if (edited !== undefined)
        open.field = { ...open.field, editor: oneLine(edited) };
    return { open };
}
