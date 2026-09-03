import type { Palette, RGB } from "../../ui/theme.ts";
import { row, wrap } from "../../ui/render.ts";
import { transcriptLead, transcriptWidth } from "../transcript-grammar.ts";
import type { NoticeBlock, NoticeTone } from "./types.ts";

export function renderNotice(block: NoticeBlock, width: number, pal: Palette): string[] {
  const fg: Record<NoticeTone, RGB> = {
    info: pal.ink.muted,
    warn: pal.ink.attention,
    error: pal.ink.removed,
  };
  const mark = block.tone === "error" ? "×" : block.tone === "warn" ? "!" : "·";
  return [
    "",
    ...wrap(block.text, transcriptWidth(width)).map((line, index) =>
      row(
        width,
        [
          ...transcriptLead(width, index === 0
            ? { text: mark, fg: fg[block.tone], bold: block.tone !== "info" }
            : undefined),
          { text: line, fg: fg[block.tone] },
        ],
      )
    ),
  ];
}
