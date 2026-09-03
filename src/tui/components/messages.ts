import type { Palette } from "../../ui/theme.ts";
import { trailingText } from "../../text-boundary.ts";
import { row } from "../../ui/render.ts";
import { markdown } from "../../ui/markdown.ts";
import { transcriptLead, transcriptWidth } from "../transcript-grammar.ts";
import type { AnswerBlock, ReasoningBlock, UserBlock } from "./types.ts";

export const REASONING_PREVIEW_ROWS = 3;
const MIN_REASONING_PREVIEW_CHARS = 4_096;
const REASONING_PREVIEW_OVERSCAN = 12;

export function renderUser(block: UserBlock, width: number, pal: Palette): string[] {
  const inner = transcriptWidth(width);
  const content = markdown(block.text, inner, pal, inner);
  return [
    "",
    ...content.map((line, index) => row(width, [
      ...transcriptLead(width, index === 0
        ? { text: "❯", fg: pal.accent, bold: true }
        : undefined),
      ...line.segs,
    ])),
  ];
}

export function renderAnswer(block: AnswerBlock, width: number, pal: Palette): string[] {
  const inner = transcriptWidth(width);
  return [
    "",
    ...markdown(block.text, inner, pal, inner).map((line) =>
      row(width, [...transcriptLead(width), ...line.segs])
    ),
  ];
}

export function renderReasoning(
  block: ReasoningBlock,
  width: number,
  pal: Palette,
  context: { continues?: boolean } = {},
): string[] {
  const inner = transcriptWidth(width);
  // Expanding a live stream is deferred until it is sealed. Re-parsing an
  // ever-growing full thought on every token makes the whole TUI stall.
  const expanded = block.expanded === true && block.live !== true;
  const source = !expanded
    ? reasoningPreviewSource(block.text, inner)
    : { text: block.text, truncated: false };
  const content = markdown(source.text, inner, pal, inner);
  const visible = expanded ? content : content.slice(-REASONING_PREVIEW_ROWS);

  return [
    ...(context.continues === true ? [] : [""]),
    ...visible.map((line) =>
      row(
        width,
        [
          ...transcriptLead(width, { text: "│", fg: pal.rule }),
          ...line.segs.map((seg) => ({ ...seg, fg: pal.ink.dim, italic: true })),
        ],
      )
    ),
  ];
}

export function reasoningPreviewSource(
  text: string,
  width: number,
): { text: string; truncated: boolean } {
  const limit = Math.max(
    MIN_REASONING_PREVIEW_CHARS,
    width * REASONING_PREVIEW_ROWS * REASONING_PREVIEW_OVERSCAN,
  );
  if (text.length <= limit) return { text, truncated: false };

  // A compact view only needs its visible tail. The complete text remains on
  // the block for expansion after the reasoning stream is sealed.
  return { text: trailingText(text, limit), truncated: true };
}
