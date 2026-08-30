// One interaction model for every terminal selector.
import { row } from "../ui/render.js";
import { elide } from "../ui/width.js";
import { menuWindow, renderMenuRows } from "./components/menu.js";
const WINDOW = 6;
export function move(picker, step) {
    const shown = matches(picker);
    if (shown.length === 0)
        return picker;
    const at = Math.max(0, shown.findIndex((entry) => entry.index === picker.index));
    return { ...picker, index: shown[((at + step) % shown.length + shown.length) % shown.length].index };
}
export function edge(picker, end) {
    const shown = matches(picker);
    const found = end === "home" ? shown[0] : shown.at(-1);
    return found === undefined ? picker : { ...picker, index: found.index };
}
export function page(picker, direction, rows = WINDOW) {
    return move(picker, direction * Math.max(1, rows));
}
export function type(picker, text) {
    if (picker.searchable !== true || text === "")
        return picker;
    return withQuery(picker, `${picker.query ?? ""}${text}`);
}
export function backspace(picker) {
    if (picker.searchable !== true || (picker.query ?? "") === "")
        return picker;
    return withQuery(picker, Array.from(picker.query ?? "").slice(0, -1).join(""));
}
export function clear(picker) {
    return withQuery(picker, "");
}
export function selected(picker) {
    return matches(picker).some((entry) => entry.index === picker.index) ? picker.index : undefined;
}
export function byKey(picker, text) {
    const typed = text.trim().toLowerCase();
    if (typed === "")
        return undefined;
    const digit = Number(typed);
    if (Number.isInteger(digit) && digit >= 1 && digit <= picker.options.length)
        return digit - 1;
    const found = picker.options.findIndex((option) => option.key === typed);
    return found === -1 ? undefined : found;
}
export function panel(picker, width, pal, maxRows = WINDOW + 4) {
    const found = matches(picker);
    const fixed = 2 + (picker.description === undefined ? 0 : 1) + (picker.searchable === true ? 1 : 0);
    const optionRoom = Math.max(1, Math.min(WINDOW, maxRows - fixed));
    const selectedAt = Math.max(0, found.findIndex((entry) => entry.index === picker.index));
    const { first, last } = menuWindow(found.length, selectedAt, optionRoom);
    const shown = found.slice(first, last);
    const options = shown.length === 0
        ? [row(width, [{ text: "  no matches", fg: pal.ink.muted }])]
        : renderMenuRows(shown.map(({ option, index }) => ({
            label: option.label,
            hint: option.hint,
            selected: index === picker.index,
        })), width, pal);
    const progress = found.length > shown.length || picker.searchable === true
        ? `${found.length === 0 ? "0" : `${first + 1}–${first + shown.length}`} / ${found.length}` +
            (found.length === picker.options.length ? "" : ` · ${picker.options.length} total`) + " · "
        : "";
    const footer = picker.footer ?? "Enter to select · Esc to close";
    return [
        row(width, picker.title, picker.right === undefined ? [] : [{ text: picker.right, fg: pal.ink.muted }]),
        ...(picker.description === undefined
            ? []
            : [row(width, [{ text: picker.description, fg: pal.ink.muted }])]),
        ...(picker.searchable === true
            ? [
                row(width, [
                    { text: "  filter  ", fg: pal.ink.muted },
                    { text: picker.query === "" || picker.query === undefined ? "type to search" : picker.query, fg: pal.ink.bright },
                ]),
            ]
            : []),
        ...options,
        row(width, [{ text: `  ${elide(progress + footer, Math.max(1, width - 2))}`, fg: pal.ink.muted }]),
    ];
}
export function heading(label, about, pal) {
    return [
        { text: `${label}  `, fg: pal.accent, bold: true },
        { text: about, fg: pal.ink.fg },
    ];
}
function matches(picker) {
    const query = (picker.query ?? "").trim().toLocaleLowerCase();
    return picker.options.flatMap((option, index) => {
        const haystack = `${option.label} ${option.hint ?? ""}`.toLocaleLowerCase();
        return query === "" || haystack.includes(query) ? [{ option, index }] : [];
    });
}
function withQuery(picker, query) {
    const next = { ...picker, query };
    return { ...next, index: matches(next)[0]?.index ?? 0 };
}
