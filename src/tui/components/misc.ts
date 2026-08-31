import type { Palette, RGB } from "../../ui/theme.ts";
import { row, wrap } from "../../ui/render.ts";
import type { NoticeBlock, NoticeTone } from "./types.ts";

export function renderNotice(block: NoticeBlock, width: number, pal: Palette): string[] {
  const fg: Record<NoticeTone, RGB> = {
    info: pal.ink.muted,
    warn: pal.ink.attention,
    error: pal.ink.removed,
  };
  const mark = block.tone === "error" ? "× " : block.tone === "warn" ? "! " : "· ";
  return [
    "",
    ...wrap(block.text, Math.max(1, width - 3)).map((line, index) =>
      row(
        width,
        [
          { text: index === 0 ? mark : "  ", fg: fg[block.tone], bold: index === 0 },
          { text: line, fg: fg[block.tone] },
        ],
        [],
        undefined,
        1,
      )
    ),
  ];
}
