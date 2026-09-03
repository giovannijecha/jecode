// Raw bytes from a terminal, turned into key events.
//
// Two things make this stateful rather than a pure function. An escape
// sequence can be split across reads, so an incomplete one is held rather than
// guessed at; and a bracketed paste arrives as a delimited run that must not be
// interpreted key by key, or a pasted newline submits half the paste.

import { MAX_PROMPT_CODE_UNITS } from "../input-boundary.ts";

const ESC = String.fromCharCode(27);
const BS = String.fromCharCode(8);
const DEL = String.fromCharCode(127);
const PASTE_START = "[200~";
const PASTE_END = "[201~";

/** One SGR mouse report, in zero-based frame coordinates. */
export type Pointer = {
  action: "press" | "release" | "move" | "wheel";
  button: "left" | "middle" | "right" | "none";
  wheel?: "up" | "down";
  col: number;
  row: number;
};

export type Key = {
  /** "char" and "paste" carry text, "pointer" carries a report. */
  name: string;
  text: string;
  ctrl: boolean;
  pointer?: Pointer;
};

// SGR mouse reporting (?1006) rather than the original X10 encoding: the old
// one packs the coordinates into single bytes and simply stops being able to
// say where the pointer is past column 223.
const MOUSE = /^\[<(\d+);(\d+);(\d+)([Mm])/;
const CSI_SEQUENCE = /^\[[0-?]*[ -/]*[@-~]/;
const SS3_SEQUENCE = /^O[ -~]/;
const BUTTONS = ["left", "middle", "right", "none"] as const;

// Both the normal and the application-cursor forms, because a terminal sends
// either depending on the mode it thinks it is in.
const SEQUENCES: Record<string, string> = {
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
  "[3;5~": "deletewordright",
  "[5~": "pageup",
  "[6~": "pagedown",
  "[1;5C": "wordright",
  "[1;5D": "wordleft",
  // Explicit modified-key forms used by terminals with CSI-u support.
  "[127;5u": "deletewordleft",
  "[8;5u": "deletewordleft",
  // Readline-compatible Alt bindings. VS Code also sends Alt+D for
  // Ctrl+Delete and Ctrl+W for Ctrl+Backspace in its integrated terminal.
  d: "deletewordright",
  [DEL]: "deletewordleft",
  "[Z": "backtab",
  // alt+enter, which arrives as an escape followed by the return itself. It is
  // how a multi-line message gets written when enter is what sends one.
  "\r": "newline",
};

const CONTROL: Record<string, string> = {
  "\r": "enter",
  "\n": "enter",
  "\t": "tab",
  [BS]: "backspace",
  [DEL]: "backspace",
};

export type DecoderOptions = {
  /** Windows Terminal's default mode uses DEL, then BS when Ctrl is held. */
  ctrlBackspaceIsBs?: boolean;
};

export type Decoder = {
  push(chunk: string): Key[];
  /** Emit whatever is still held. A lone escape only resolves this way. */
  flush(): Key[];
};

export function decoder(options: DecoderOptions = {}): Decoder {
  let held = "";
  let pasting = false;
  let pasted = "";
  let pasteTooLong = false;

  const appendPaste = (text: string): void => {
    if (pasteTooLong || text === "") return;
    if (text.length > MAX_PROMPT_CODE_UNITS - pasted.length) {
      pasted = "";
      pasteTooLong = true;
      return;
    }
    pasted += text;
  };

  const drain = (final: boolean): Key[] => {
    const keys: Key[] = [];

    while (held !== "") {
      if (pasting) {
        const end = held.indexOf(ESC + PASTE_END);
        if (end === -1) {
          const interrupt = firstInterrupt(held);
          if (interrupt !== -1) {
            // A missing bracketed-paste terminator must not trap the decoder
            // forever. Ctrl+C/Ctrl+D are emergency exits: discard the partial
            // paste, then let the normal control-key path handle the byte.
            held = held.slice(interrupt);
            pasted = "";
            pasteTooLong = false;
            pasting = false;
            continue;
          }
          // Hold back a possible partial terminator rather than pasting it.
          const safe = held.length - PASTE_END.length - 1;
          if (safe > 0) {
            appendPaste(held.slice(0, safe));
            held = held.slice(safe);
          }
          break;
        }
        appendPaste(held.slice(0, end));
        held = held.slice(end + PASTE_END.length + 1);
        pasting = false;
        keys.push(pasteTooLong
          ? { name: "input_limit", text: "", ctrl: false }
          : { name: "paste", text: pasted, ctrl: false });
        pasted = "";
        pasteTooLong = false;
        continue;
      }

      const ch = held[0] as string;

      if (ch === ESC) {
        const rest = held.slice(1);

        if (rest === "" && !final) break;
        if (rest.startsWith(PASTE_START)) {
          held = rest.slice(PASTE_START.length);
          pasting = true;
          pasteTooLong = false;
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
          keys.push({ name: SEQUENCES[match] as string, text: "", ctrl: false });
          continue;
        }
        const unbound = completeTerminalSequence(rest);
        if (unbound !== undefined) {
          // Terminals have many optional keys and mode reports. An unbound but
          // complete CSI/SS3 sequence is terminal protocol, never editor text.
          held = rest.slice(unbound.length);
          continue;
        }
        if (!final && couldGrow(rest)) break;

        held = held.slice(1);
        keys.push({ name: "escape", text: "", ctrl: false });
        continue;
      }

      const named = ch === BS && options.ctrlBackspaceIsBs === true
        ? "deletewordleft"
        : CONTROL[ch];
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
        const next = held[end] as string;
        if ((next.codePointAt(0) ?? 0) < 0x20 || next === ESC || next === DEL) break;
        end++;
      }
      keys.push({ name: "char", text: held.slice(0, end), ctrl: false });
      held = held.slice(end);
    }

    return keys;
  };

  return {
    push(chunk: string): Key[] {
      // The usual printable run needs no protocol buffering. This also rejects
      // one unbracketed paste atomically before copying it into `held`.
      if (held === "" && !pasting && printable(chunk)) {
        return [chunk.length > MAX_PROMPT_CODE_UNITS
          ? { name: "input_limit", text: "", ctrl: false }
          : { name: "char", text: chunk, ctrl: false }];
      }

      const keys: Key[] = [];
      const chunkSize = 64 * 1_024;
      for (let from = 0; from < chunk.length;) {
        let to = Math.min(from + chunkSize, chunk.length);
        if (
          to < chunk.length &&
          isHighSurrogate(chunk.charCodeAt(to - 1)) &&
          isLowSurrogate(chunk.charCodeAt(to))
        ) {
          to++;
        }
        const part = chunk.slice(from, to);
        from = to;
        if (part.length > MAX_PROMPT_CODE_UNITS - held.length) {
          held = "";
          pasted = "";
          pasting = false;
          pasteTooLong = false;
          keys.push({ name: "input_limit", text: "", ctrl: false });
          break;
        }
        held += part;
        keys.push(...drain(false));
      }
      return keys;
    },
    flush(): Key[] {
      return drain(true);
    },
  };
}

function isHighSurrogate(code: number): boolean {
  return code >= 0xd800 && code <= 0xdbff;
}

function isLowSurrogate(code: number): boolean {
  return code >= 0xdc00 && code <= 0xdfff;
}

function printable(text: string): boolean {
  for (let index = 0; index < text.length; index++) {
    const char = text[index] as string;
    if ((char.codePointAt(0) ?? 0) < 0x20 || char === ESC || char === DEL) return false;
  }
  return text !== "";
}

function firstInterrupt(text: string): number {
  const ctrlC = text.indexOf(String.fromCharCode(3));
  const ctrlD = text.indexOf(String.fromCharCode(4));
  if (ctrlC === -1) return ctrlD;
  if (ctrlD === -1) return ctrlC;
  return Math.min(ctrlC, ctrlD);
}

function matchSequence(rest: string): string | undefined {
  for (const seq of Object.keys(SEQUENCES)) {
    if (rest.startsWith(seq)) return seq;
  }
  return undefined;
}

function completeTerminalSequence(rest: string): string | undefined {
  return CSI_SEQUENCE.exec(rest)?.[0] ?? SS3_SEQUENCE.exec(rest)?.[0];
}

/** Whether `rest` is still a viable prefix of something we know. */
function couldGrow(rest: string): boolean {
  if (PASTE_START.startsWith(rest)) return true;
  if (/^\[[0-?]*[ -/]*$/.test(rest)) return true;
  // A mouse report has no fixed length, so it grows until its final letter.
  if (/^\[<?\d*;?\d*;?\d*$/.test(rest)) return true;
  return Object.keys(SEQUENCES).some((seq) => seq.startsWith(rest));
}

function pointerKey(match: RegExpExecArray): Key {
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
