// Project semantic blocks into a compact transcript for pipes.

import type { Palette } from "./ui/theme.ts";
import type { Block } from "./tui/blocks.ts";
import { render } from "./tui/blocks.ts";

export function renderBatch(block: Block, width: number, pal: Palette): string[] {
  const rows: string[] = [];
  let lastWasBlank = false;

  for (const rendered of render(block, width, pal)) {
    const line = rendered.trimEnd();
    const blank = line === "";
    if (blank && lastWasBlank) continue;
    rows.push(line);
    lastWasBlank = blank;
  }

  while (rows.at(-1) === "") rows.pop();
  return rows;
}
