// Raw bytes from a terminal, turned into key events.
//
// Two things make this stateful rather than a pure function. An escape
// sequence can be split across reads, so an incomplete one is held rather than
// guessed at; and a bracketed paste arrives as a delimited run that must not be
// interpreted key by key, or a pasted newline submits half the paste.
const ESC = String.fromCharCode(27);
const DEL = String.fromCharCode(127);
const PASTE_START = "[200~";
const PASTE_END = "[201~";
// SGR mouse reporting (?1006) rather than the original X10 encoding: the old
// one packs the coordinates into single bytes and simply stops being able to
// say where the pointer is past column 223.
const MOUSE = /^\[<(\d+);(\d+);(\d+)([Mm])/;
const BUTTONS = ["left", "middle", "right", "none"];
// Both the normal and the application-cursor forms, because a terminal sends
// either depending on the mode it thinks it is in.
const SEQUENCES = {
    "[A": "up",
    "[B": "down",
    "[C": "right",
    "[D": "left",
    OA: "up",
    OB: "down",
    OC: "right",
    OD: "left",
    "[H": "home",
    "[F": "end",
    OH: "home",
    OF: "end",
    "[1~": "home",
    "[4~": "end",
    "[7~": "home",
    "[8~": "end",
    "[3~": "delete",
    "[5~": "pageup",
    "[6~": "pagedown",
    "[1;5C": "wordright",
    "[1;5D": "wordleft",
    "[Z": "backtab",
    // alt+enter, which arrives as an escape followed by the return itself. It is
    // how a multi-line message gets written when enter is what sends one.
    "\r": "newline",
};
const CONTROL = {
    "\r": "enter",
    "\n": "enter",
    "\t": "tab",
    "\b": "backspace",
    [DEL]: "backspace",
};
export function decoder() {
    let held = "";
    let pasting = false;
    let pasted = "";
    const drain = (final) => {
        const keys = [];
        while (held !== "") {
            if (pasting) {
                const end = held.indexOf(ESC + PASTE_END);
                if (end === -1) {
                    // Hold back a possible partial terminator rather than pasting it.
                    const safe = held.length - PASTE_END.length - 1;
                    if (safe > 0) {
                        pasted += held.slice(0, safe);
                        held = held.slice(safe);
                    }
                    break;
                }
                pasted += held.slice(0, end);
                held = held.slice(end + PASTE_END.length + 1);
                pasting = false;
                keys.push({ name: "paste", text: pasted, ctrl: false });
                pasted = "";
                continue;
            }
            const ch = held[0];
            if (ch === ESC) {
                const rest = held.slice(1);
                if (rest === "" && !final)
                    break;
                if (rest.startsWith(PASTE_START)) {
                    held = rest.slice(PASTE_START.length);
                    pasting = true;
                    continue;
                }
                const mouse = MOUSE.exec(rest);
                if (mouse !== null) {
                    held = rest.slice(mouse[0].length);
                    keys.push(pointerKey(mouse));
                    continue;
                }
                const match = matchSequence(rest);
                if (match !== undefined) {
                    held = rest.slice(match.length);
                    keys.push({ name: SEQUENCES[match], text: "", ctrl: false });
                    continue;
                }
                if (!final && couldGrow(rest))
                    break;
                held = held.slice(1);
                keys.push({ name: "escape", text: "", ctrl: false });
                continue;
            }
            const named = CONTROL[ch];
            if (named !== undefined) {
                held = held.slice(1);
                keys.push({ name: named, text: "", ctrl: false });
                continue;
            }
            // C0 controls other than the named ones are ctrl+letter.
            const code = ch.codePointAt(0) ?? 0;
            if (code < 0x20) {
                held = held.slice(1);
                keys.push({ name: String.fromCharCode(code + 0x60), text: "", ctrl: true });
                continue;
            }
            // Printable runs travel together: an unbracketed paste arrives this way,
            // and inserting it one character at a time is pure overhead.
            let end = 1;
            while (end < held.length) {
                const next = held[end];
                if ((next.codePointAt(0) ?? 0) < 0x20 || next === ESC || next === DEL)
                    break;
                end++;
            }
            keys.push({ name: "char", text: held.slice(0, end), ctrl: false });
            held = held.slice(end);
        }
        return keys;
    };
    return {
        push(chunk) {
            held += chunk;
            return drain(false);
        },
        flush() {
            return drain(true);
        },
    };
}
function matchSequence(rest) {
    for (const seq of Object.keys(SEQUENCES)) {
        if (rest.startsWith(seq))
            return seq;
    }
    return undefined;
}
/** Whether `rest` is still a viable prefix of something we know. */
function couldGrow(rest) {
    if (PASTE_START.startsWith(rest))
        return true;
    // A mouse report has no fixed length, so it grows until its final letter.
    if (/^\[<?\d*;?\d*;?\d*$/.test(rest))
        return true;
    return Object.keys(SEQUENCES).some((seq) => seq.startsWith(rest));
}
function pointerKey(match) {
    const code = Number(match[1]);
    const col = Number(match[2]) - 1;
    const row = Number(match[3]) - 1;
    const released = match[4] === "m";
    const wheeling = (code & 64) !== 0;
    const action = wheeling ? "wheel" : released ? "release" : (code & 32) !== 0 ? "move" : "press";
    return {
        name: "pointer",
        text: "",
        ctrl: (code & 16) !== 0,
        pointer: {
            action,
            button: wheeling ? "none" : (BUTTONS[code & 3] ?? "none"),
            wheel: wheeling ? ((code & 1) === 0 ? "up" : "down") : undefined,
            col,
            row,
        },
    };
}
