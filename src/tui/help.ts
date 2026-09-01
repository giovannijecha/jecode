// The compact, non-persistent keyboard reference shown inside the dock.

import type { Palette } from "../ui/theme.ts";
import { row } from "../ui/render.ts";
import { textWidth } from "../ui/width.ts";

const CONTROL_WIDTH = 18;

const CONTROLS: readonly { key: string; description: string }[] = [
  { key: "up / down", description: "move through menus or history" },
  { key: "left / right", description: "move cursor or change a value" },
  { key: "ctrl+left / right", description: "move cursor by word" },
  { key: "ctrl+backspace/del", description: "delete a word" },
  { key: "enter / tab", description: "select or send · complete" },
  { key: "alt+enter", description: "insert a new line" },
  { key: "esc", description: "close UI or interrupt work" },
  { key: "ctrl+c", description: "interrupt work or exit" },
  { key: "ctrl+o", description: "toggle reasoning or tool details" },
  { key: "wheel / pgup/dn", description: "scroll the transcript" },
  { key: "ctrl+l", description: "redraw the screen" },
];

export function panel(width: number, pal: Palette, maxRows = CONTROLS.length + 1): string[] {
  const heading = row(
    width,
    [
      { text: "help  ", fg: pal.accent, bold: true },
      { text: "keyboard controls", fg: pal.ink.fg },
    ],
    [{ text: "esc close", fg: pal.ink.dim }],
  );
  const controls = CONTROLS.map((control) => {
    const gap = Math.max(2, CONTROL_WIDTH - textWidth(control.key));
    return row(width, [
      { text: control.key, fg: pal.ink.bright, bold: true },
      { text: " ".repeat(gap) },
      { text: control.description, fg: pal.ink.muted },
    ]);
  });
  return [heading, ...controls].slice(0, Math.max(1, maxRows));
}
