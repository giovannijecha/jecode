// Semantic transcript blocks routed to small, owned production components.

import type { Palette } from "../ui/theme.ts";
import { renderAnswer, renderReasoning, renderUser } from "./components/messages.ts";
import { renderList, renderNotice } from "./components/misc.ts";
import { renderTool } from "./components/tool.ts";
import type { Block } from "./components/types.ts";

export type {
  AnswerBlock,
  Block,
  Detail,
  Emphasis,
  ListBlock,
  NoticeBlock,
  NoticeTone,
  ReasoningBlock,
  ToolBlock,
  ToolTone,
  UserBlock,
} from "./components/types.ts";

export function render(block: Block, width: number, pal: Palette): string[] {
  switch (block.kind) {
    case "user":
      return renderUser(block, width, pal);
    case "answer":
      return renderAnswer(block, width, pal);
    case "reasoning":
      return renderReasoning(block, width, pal);
    case "tool":
      return renderTool(block, width, pal);
    case "notice":
      return renderNotice(block, width, pal);
    case "list":
      return renderList(block, width, pal);
  }
}

export function renderAll(blocks: readonly Block[], width: number, pal: Palette): string[] {
  return blocks.flatMap((block) => render(block, width, pal));
}
