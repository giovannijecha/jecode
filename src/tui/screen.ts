// The terminal, taken over and given back.
//
// A TUI owns the screen, and every takeover has to be undone — on a clean
// exit, on a crash, on a signal. The restore is registered before the first
// escape is written, never after: a process that dies holding raw mode leaves
// the user with a shell that does not echo.

import { CSI } from "../ui/render.ts";

const ALT_ON = `${CSI}?1049h`;
const ALT_OFF = `${CSI}?1049l`;
const CURSOR_HIDE = `${CSI}?25l`;
const CURSOR_SHOW = `${CSI}?25h`;
const PASTE_ON = `${CSI}?2004h`;
const PASTE_OFF = `${CSI}?2004l`;
// Auto-wrap off: every row is positioned absolutely and painted to the full
// width, and a full-width write on the last row would otherwise scroll the
// screen out from under the frame.
const WRAP_OFF = `${CSI}?7l`;
const WRAP_ON = `${CSI}?7h`;
// Button events without drag (?1000) reported in SGR form (?1006). Wheel events
// come with them, and the wheel is the reason to ask: a transcript that can
// only be scrolled by keyboard reads as a transcript that cannot be scrolled.
const MOUSE_ON = `${CSI}?1000h${CSI}?1006h`;
const MOUSE_OFF = `${CSI}?1006l${CSI}?1000l`;
// The caret, as a blinking filled cell rather than a hairline (DECSCUSR). A
// bar is the default because a bar is what a text field wants; this is not a
// text field, it is a screen jecode drew, and the block is the only mark on it
// wide enough to find at a glance. Blinking on purpose: on a screen with no
// other chrome around the input, a steady block is one more static glyph, and
// motion is what says the thing is waiting for you. `0` puts the terminal back
// on whatever the user chose.
const CURSOR_BLOCK = `${CSI}1 q`;
const CURSOR_STEADY = `${CSI}2 q`;
const CURSOR_RESET = `${CSI}0 q`;
// Synchronized output: the terminal is told to hold the screen until the frame
// is complete. Without it a repaint mid-refresh is drawn half-old and
// half-new, which is what tearing during a stream actually is.
const SYNC_BEGIN = `${CSI}?2026h`;
const SYNC_END = `${CSI}?2026l`;

export type Size = { rows: number; cols: number };

let active = false;
let restoreHandlersRegistered = false;

export function interactive(): boolean {
  return process.stdin.isTTY === true && process.stdout.isTTY === true;
}

export function size(): Size {
  return {
    rows: Math.max(1, process.stdout.rows ?? 24),
    cols: Math.max(1, process.stdout.columns ?? 80),
  };
}

export function write(text: string): void {
  process.stdout.write(text);
}

export function outputReady(): boolean {
  return !process.stdout.writableNeedDrain;
}

export function onDrain(handler: () => void): () => void {
  process.stdout.on("drain", handler);
  return () => { process.stdout.off("drain", handler); };
}

export function enter(reducedMotion = false): void {
  if (active) return;
  active = true;

  registerRestoreHandlers();

  write(ALT_ON + WRAP_OFF + CURSOR_HIDE + (reducedMotion ? CURSOR_STEADY : CURSOR_BLOCK) + PASTE_ON + MOUSE_ON);
  process.stdin.setRawMode(true);
  process.stdin.setEncoding("utf8");
  process.stdin.resume();
}

export function leave(): void {
  if (!active) return;
  active = false;

  if (process.stdin.isTTY === true) process.stdin.setRawMode(false);
  process.stdin.pause();
  write(MOUSE_OFF + PASTE_OFF + CURSOR_RESET + CURSOR_SHOW + WRAP_ON + ALT_OFF);
}

export function setReducedMotion(reducedMotion: boolean): void {
  if (active) write(reducedMotion ? CURSOR_STEADY : CURSOR_BLOCK);
}

function registerRestoreHandlers(): void {
  if (restoreHandlersRegistered) return;
  restoreHandlersRegistered = true;
  process.on("exit", leave);
  process.on("uncaughtExceptionMonitor", leave);
}

export function onResize(handler: () => void): () => void {
  process.stdout.on("resize", handler);
  return () => {
    process.stdout.off("resize", handler);
  };
}

export function onInput(handler: (chunk: string) => void): () => void {
  process.stdin.on("data", handler as (chunk: unknown) => void);
  return () => {
    process.stdin.off("data", handler as (chunk: unknown) => void);
  };
}

export const CURSOR = { hide: CURSOR_HIDE, show: CURSOR_SHOW };
export const SYNC = { begin: SYNC_BEGIN, end: SYNC_END };
