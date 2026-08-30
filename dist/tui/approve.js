// The permission question: what is being asked, and the answers on offer.
//
// A permission prompt is the one moment the agent hands control back, so it is
// worth more than a line of prose and a key to guess at. It is a menu — the
// generic one — with an attention title and three answers written out.
// Nothing is approved by a key nobody meant to press.
/**
 * The narrow permission represented by "always".
 *
 * File-changing tools share a grant for one displayed path. Shell grants are
 * tied to the exact command line. Unknown dangerous tools fall back to their
 * exact, stable input rather than silently inheriting a tool-wide grant.
 */
export function scopeFor(call) {
    const path = typeof call.input.path === "string" ? call.input.path : undefined;
    if ((call.name === "write_file" || call.name === "edit_file") && path !== undefined) {
        return { key: `file\0${path}`, label: `changes to ${path}`, summary: `file changes · ${path}` };
    }
    const command = typeof call.input.command === "string" ? call.input.command : undefined;
    if (call.name === "run_command" && command !== undefined) {
        return { key: `command\0${command}`, label: "this exact command", summary: `command · ${command}` };
    }
    return {
        key: `${call.name}\0${stable(call.input)}`,
        label: "this exact call",
        summary: `${call.name} · ${target(call.input)}`,
    };
}
export function promptFor(call, target, pal) {
    const scope = scopeFor(call);
    const options = [
        { label: `Run this ${call.name === "edit_file" ? "edit" : "call"} once`, hint: "enter", key: "y" },
        { label: `Allow ${scope.label} this session`, hint: "a", key: "a" },
        { label: "Deny and add feedback", hint: "esc", key: "n" },
    ];
    return {
        title: [
            { text: "Permission required", fg: pal.ink.attention, bold: true },
            { text: `  ${call.name}`, fg: pal.ink.muted },
            ...(target === "" ? [] : [{ text: ` ${target}`, fg: pal.ink.muted }]),
        ],
        description: "Review the exact action above, then choose.",
        footer: "Enter to select · Esc to deny",
        options,
        index: 0,
    };
}
function stable(value) {
    if (Array.isArray(value))
        return `[${value.map(stable).join(",")}]`;
    if (value !== null && typeof value === "object") {
        return `{${Object.entries(value)
            .sort(([a], [b]) => a.localeCompare(b))
            .map(([key, item]) => `${JSON.stringify(key)}:${stable(item)}`)
            .join(",")}}`;
    }
    return JSON.stringify(value);
}
function target(input) {
    const value = input.path ?? input.command;
    return typeof value === "string" ? value : stable(input);
}
const ANSWERS = ["once", "always", "no"];
/** What picking row `index` meant. Anything unrecognised refuses. */
export function answerAt(index) {
    return index === undefined ? "no" : (ANSWERS[index] ?? "no");
}
