// Project semantic blocks into a compact transcript for pipes.

import type { Palette } from "./ui/theme.ts";
import type { Block } from "./tui/blocks.ts";
import { renderAnswer, renderReasoning, renderUser } from "./tui/components/messages.ts";
import { renderNotice } from "./tui/components/misc.ts";
import { renderDetail } from "./tui/components/tool-evidence.ts";
import { stateInk, stateMark } from "./tui/components/tool-motion.ts";
import { toolDuration } from "./duration.ts";
import { row } from "./ui/render.ts";

export function renderBatch(block: Block, width: number, pal: Palette): string[] {
  const rows: string[] = [];
  let lastWasBlank = false;

  for (const rendered of batchRows(block, width, pal)) {
    const line = rendered.trimEnd();
    const blank = line === "";
    if (blank && lastWasBlank) continue;
    rows.push(line);
    lastWasBlank = blank;
  }

  while (rows.at(-1) === "") rows.pop();
  return rows;
}

function batchRows(block: Block, width: number, pal: Palette): string[] {
  switch (block.kind) {
    case "answer": return renderAnswer(block, width, pal);
    case "reasoning": return renderReasoning(block, width, pal);
    case "user": return renderUser(block, width, pal);
    case "notice": return renderNotice(block, width, pal);
    case "tool": {
      // Pipes cannot expand records or repaint a running connector.
      const mark = stateMark(block);
      const result = [block.right, block.durationMs === undefined ? "" : toolDuration(block.durationMs)]
        .filter(Boolean).join(" · ");
      return ["", row(width, [
        { text: `${mark} `, fg: stateInk(block, pal) },
        { text: block.name, fg: pal.ink.bright },
        { text: block.target === "" ? "" : `  ${block.target}`, fg: pal.technical },
      ], [{ text: result, fg: pal.ink.muted }]),
      ...(block.body ?? []).map((detail) => renderDetail(detail, block.tone, width, pal))];
    }
  }
}
