// Text that is safe to hand to a terminal.
//
// Styling escapes are owned by the renderer. Everything coming from a model,
// tool, filename, error, or paste is data, so control characters are made
// visible before layout as well as before paint. This keeps terminal state and
// cell measurement in agreement.

const CONTROL_PICTURE = 0x2400;
const DELETE_PICTURE = String.fromCodePoint(0x2421);
const TAB = "  ";

export type TerminalTextOptions = {
  /** Keep line feeds while a multiline component is still splitting rows. */
  multiline?: boolean;
};

export function terminalText(text: string, options: TerminalTextOptions = {}): string {
  let safe = "";

  for (const char of text) {
    const code = char.codePointAt(0);
    if (code === undefined) continue;

    if (code === 0x0a && options.multiline === true) {
      safe += "\n";
    } else if (code === 0x09) {
      safe += TAB;
    } else if (code <= 0x1f) {
      safe += String.fromCodePoint(CONTROL_PICTURE + code);
    } else if (code === 0x7f) {
      safe += DELETE_PICTURE;
    } else if (code >= 0x80 && code <= 0x9f) {
      safe += `\\u${code.toString(16).padStart(4, "0")}`;
    } else if (isBidiControl(code)) {
      safe += `\\u${code.toString(16).padStart(4, "0")}`;
    } else {
      safe += char;
    }
  }

  return safe;
}

function isBidiControl(code: number): boolean {
  return code === 0x061c ||
    (code >= 0x200e && code <= 0x200f) ||
    (code >= 0x202a && code <= 0x202e) ||
    (code >= 0x2066 && code <= 0x2069);
}
