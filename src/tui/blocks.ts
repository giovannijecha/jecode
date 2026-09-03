// Semantic transcript blocks routed to small, owned production components.

import type { Palette } from "../ui/theme.ts";
import type { ToolMotion } from "./motion.ts";
import { renderAnswer, renderReasoning, renderUser } from "./components/messages.ts";
import { renderNotice } from "./components/misc.ts";
import { renderTool } from "./components/tool.ts";
import type { Block } from "./components/types.ts";

export type RenderContext = {
  previous?: Block;
  now?: number;
  motion?: ToolMotion;
  reducedMotion?: boolean;
};

export type {
  AnswerBlock,
  Block,
  Detail,
  Emphasis,
  NoticeBlock,
  NoticeTone,
  ReasoningBlock,
  ToolBlock,
  ToolTone,
  UserBlock,
} from "./components/types.ts";

export function render(block: Block, width: number, pal: Palette, context: RenderContext = {}): string[] {
  switch (block.kind) {
    case "user":
      return renderUser(block, width, pal);
    case "answer":
      return renderAnswer(block, width, pal);
    case "reasoning":
      return renderReasoning(block, width, pal, {
        continues: context.previous?.kind === "reasoning",
        followsTool: context.previous?.kind === "tool",
      });
    case "tool":
      return renderTool(block, width, pal, {
        continues: context.previous?.kind === "tool" || context.previous?.kind === "reasoning",
        followsReasoning: context.previous?.kind === "reasoning",
        now: context.now,
        motion: context.motion,
        reducedMotion: context.reducedMotion,
      });
    case "notice":
      return renderNotice(block, width, pal);
  }
}

export function renderAll(
  blocks: readonly Block[],
  width: number,
  pal: Palette,
  context: Omit<RenderContext, "previous"> = {},
): string[] {
  return blocks.flatMap((block, index) => render(block, width, pal, {
    ...context,
    previous: blocks[index - 1],
  }));
}
