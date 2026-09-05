// Semantic transcript blocks routed to small, owned production components.

import type { Palette } from "../ui/theme.ts";
import { renderAnswer, renderReasoning, renderUser } from "./components/messages.ts";
import { renderNotice } from "./components/misc.ts";
import { renderTool } from "./components/tool.ts";
import type { Block } from "./components/types.ts";
import { insetTranscript, modelTranscriptWidth } from "./transcript-grammar.ts";

export type RenderContext = {
  previous?: Block;
  now?: number;
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
  const inner = modelTranscriptWidth(width);
  switch (block.kind) {
    case "user":
      return renderUser(block, width, pal);
    case "answer":
      return insetTranscript(renderAnswer(block, inner, pal));
    case "reasoning":
      return insetTranscript(renderReasoning(block, inner, pal, {
        continues: context.previous?.kind === "reasoning",
      }));
    case "tool":
      return insetTranscript(renderTool(block, inner, pal, {
        continues: context.previous?.kind === "reasoning",
        now: context.now,
        reducedMotion: context.reducedMotion,
      }));
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
