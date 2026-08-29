// Composing a terminal row out of styled segments. Pure: every function here
// returns a string or rows of segments, none of them write anywhere.

import type { RGB } from "./theme.ts";
import { terminalText } from "./terminal-text.ts";
import { elide, textWidth, wrapText } from "./width.ts";

// Built rather than written literally: a raw escape byte in the source is
// invisible in a diff and in a code review, which is how they survive.
const ESC = String.fromCharCode(27);
export const CSI = `${ESC}[`;
const RESET = `${CSI}0m`;
const BOLD = `${CSI}1m`;
const ITALIC = `${CSI}3m`;
const INVERSE = `${CSI}7m`;

// Truecolor is assumed wherever colour is wanted at all — every terminal that
// has shipped this decade has it. The fallback that matters is "no colour",
// which is what a pipe and NO_COLOR both get.
const terminalColor =
  process.stdout.isTTY === true &&
  process.env["NO_COLOR"] === undefined &&
  process.env["TERM"] !== "dumb";
let colorEnabled = terminalColor;

export function configureColor(enabled: boolean): void {
  colorEnabled = terminalColor && enabled;
}

export type Seg = {
  text: string;
  fg?: RGB;
  bg?: RGB;
  bold?: boolean;
  italic?: boolean;
  inverse?: boolean;
};

export function paint(seg: Seg): string {
  const text = terminalText(seg.text);
  if (!colorEnabled) return text;

  let out = "";
  if (seg.bold === true) out += BOLD;
  if (seg.italic === true) out += ITALIC;
  if (seg.inverse === true) out += INVERSE;
  if (seg.fg !== undefined) out += `${CSI}38;2;${seg.fg[0]};${seg.fg[1]};${seg.fg[2]}m`;
  if (seg.bg !== undefined) out += `${CSI}48;2;${seg.bg[0]};${seg.bg[1]};${seg.bg[2]}m`;
  return out === "" ? text : `${out}${text}${RESET}`;
}

/** Cells the segments take on screen — never their character count. */
export function plainLen(segs: readonly Seg[]): number {
  return segs.reduce((n, seg) => n + textWidth(terminalText(seg.text)), 0);
}

export function columns(): number {
  const width = process.stdout.columns;
  return typeof width === "number" && width >= 40 ? width : 80;
}

/**
 * The margin every row keeps from the edges of the terminal: none.
 *
 * A window is as wide as its owner made it, and the content column is the whole
 * of it. Everything then shares one left edge and one right edge — prose, the
 * ground behind a message, the rules around the composer, the
 * footer — so nothing on screen is wider than the text inside it. It stays a
 * constant because it is a decision, not an assumption: one number moves the
 * whole grid inwards if it ever needs to.
 */
export const PAD = 0;

/** Keep what fits in `cols`, marking the segment that had to give way. */
export function fitSegs(segs: readonly Seg[], cols: number): Seg[] {
  const out: Seg[] = [];
  let used = 0;

  for (const seg of segs) {
    const safe = { ...seg, text: terminalText(seg.text) };
    const w = textWidth(safe.text);
    if (used + w <= cols) {
      out.push(safe);
      used += w;
      continue;
    }
    const room = cols - used;
    if (room > 0) out.push({ ...safe, text: elide(safe.text, room) });
    break;
  }

  return out;
}

/**
 * One row: left segments, a gap, right segments, optionally on a ground.
 *
 * The left column is the one that gives way when the two do not both fit —
 * the right column is a summary, and a summary that moves is worse than none.
 * A row with neither a ground nor a right column is left ragged rather than
 * padded: trailing spaces are invisible on screen but not in a paste buffer.
 */
export function row(
  width: number,
  left: readonly Seg[],
  right: readonly Seg[] = [],
  ground?: RGB,
  padX = PAD,
): string {
  const inner = Math.max(0, width - padX * 2);
  const rightW = plainLen(right);
  const gutter = right.length > 0 ? 1 : 0;
  const fitted = fitSegs(left, Math.max(0, inner - rightW - gutter));
  // An empty pad is no pad at all: a zero-width segment still costs a pair of
  // escape sequences on every row of every frame.
  const lead: Seg[] = padX === 0 ? [] : [{ text: " ".repeat(padX), bg: ground }];

  if (ground === undefined && right.length === 0) {
    // Nothing to draw is an empty row, not an indent: leading spaces are
    // invisible on screen and very visible in a paste buffer.
    if (plainLen(fitted) === 0) return "";
    return [...lead, ...fitted].map(paint).join("");
  }

  const gap = Math.max(gutter, inner - plainLen(fitted) - rightW);

  return [
    ...lead,
    ...fitted.map((seg) => ({ ...seg, bg: seg.bg ?? ground })),
    { text: " ".repeat(gap), bg: ground },
    ...right.map((seg) => ({ ...seg, bg: seg.bg ?? ground })),
    ...lead,
  ]
    .map(paint)
    .join("");
}

export function blank(width: number, ground: RGB): string {
  return paint({ text: " ".repeat(width), bg: ground });
}

/**
 * A full-width divider.
 *
 * Edge to edge, because the other thing that spans a whole row is the ground
 * behind a user's message: two elements that stop at different columns read as
 * two grids, and the eye finds the discrepancy before it finds the text.
 */
export function rule(width: number, color: RGB): string {
  return paint({ text: "─".repeat(Math.max(0, width)), fg: color });
}

export function wrap(text: string, max: number, continuation = ""): string[] {
  return wrapText(terminalText(text, { multiline: true }), max, continuation);
}

type Tok = { seg: Seg; text: string; kind: "word" | "space" | "break" };

/**
 * Word wrap that survives styling: segments flow across rows, and a row breaks
 * between words rather than between a word and its colour.
 *
 * `continuation` opens every row after the first, which is what gives a list
 * item a hanging indent instead of a second bullet.
 */
export function flow(
  segs: readonly Seg[],
  max: number,
  continuation: readonly Seg[] = [],
): Seg[][] {
  const lead = plainLen(continuation);
  const toks: Tok[] = [];

  for (const seg of segs) {
    const safe = { ...seg, text: terminalText(seg.text, { multiline: true }) };
    for (const piece of safe.text.split(/(\s+)/)) {
      if (piece === "") continue;
      if (piece.includes("\n")) toks.push({ seg: safe, text: piece, kind: "break" });
      // A space that paints a background is not whitespace, it is the edge of
      // a semantic surface and must not collapse into its neighbour.
      else if (/^\s+$/.test(piece)) {
        toks.push({ seg: safe, text: piece, kind: safe.bg === undefined ? "space" : "word" });
      } else toks.push({ seg: safe, text: piece, kind: "word" });
    }
  }

  const rows: Seg[][] = [];
  let current: Seg[] = [];
  let used = 0;
  let pending: Tok | undefined;

  const room = (): number => Math.max(1, max - (rows.length === 0 ? 0 : lead));

  const add = (tok: Tok): void => {
    const last = current[current.length - 1];
    if (last !== undefined && sameStyle(last, tok.seg)) last.text += tok.text;
    else current.push({ ...tok.seg, text: tok.text });
    used += textWidth(tok.text);
  };

  const flush = (): void => {
    rows.push(rows.length === 0 ? current : [...continuation.map(copy), ...current]);
    current = [];
    used = 0;
    pending = undefined;
  };

  for (const tok of toks) {
    if (tok.kind === "break") {
      flush();
      continue;
    }
    if (tok.kind === "space") {
      if (current.length > 0) pending = tok;
      continue;
    }

    const space = pending === undefined ? 0 : textWidth(pending.text);
    let word = tok.text;

    if (used + space + textWidth(word) > room() && current.length > 0) flush();
    else if (pending !== undefined) {
      add(pending);
      pending = undefined;
    }

    // A word wider than any row is spent across rows: with autowrap off, what
    // overflows is not ugly, it is gone.
    while (textWidth(word) > room() - used) {
      const head = clipTo(word, room() - used);
      if (head === "") break;
      add({ ...tok, text: head });
      word = word.slice(head.length);
      flush();
    }

    if (word !== "") add({ ...tok, text: word });
  }

  if (current.length > 0 || rows.length === 0) flush();
  return rows;
}

function clipTo(text: string, cols: number): string {
  let out = "";
  for (const char of text) {
    if (textWidth(out + char) > cols) break;
    out += char;
  }
  return out;
}

function copy(seg: Seg): Seg {
  return { ...seg };
}

function sameStyle(a: Seg, b: Seg): boolean {
  return (
    a.fg === b.fg &&
    a.bg === b.bg &&
    a.bold === b.bold &&
    a.italic === b.italic &&
    a.inverse === b.inverse
  );
}

/** One-line preview of a value, for echoing tool arguments back to the user. */
export function preview(value: unknown, max = 120): string {
  const text = typeof value === "string" ? value : JSON.stringify(value);
  const flat = (text ?? "").replace(/\s+/g, " ").trim();
  return elide(terminalText(flat), max);
}
