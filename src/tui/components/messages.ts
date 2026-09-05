import type { Palette } from "../../ui/theme.ts";
import { trailingText } from "../../text-boundary.ts";
import { blank, hasColor, row } from "../../ui/render.ts";
import { markdown } from "../../ui/markdown.ts";
import { terminalText } from "../../ui/terminal-text.ts";
import { splitByCells } from "../../ui/width.ts";
import { transcriptLead, transcriptWidth } from "../transcript-grammar.ts";
import type { AnswerBlock, ReasoningBlock, UserBlock } from "./types.ts";

export const REASONING_PREVIEW_ROWS = 3;
const MIN_REASONING_PREVIEW_CHARS = 4_096;
const REASONING_PREVIEW_OVERSCAN = 12;

export function renderUser(block: UserBlock, width: number, pal: Palette): string[] {
  const inner = transcriptWidth(width);
  const content = terminalText(block.text, { multiline: true }).split("\n")
    .flatMap((line) => splitByCells(line, inner));
  return [
    "",
    userEdge(width, pal, "▄"),
    ...content.map((line) => row(width, [
      ...transcriptLead(width),
      { text: line.text, fg: pal.ink.fg },
    ], [], pal.surface.subtle)),
    userEdge(width, pal, "▀"),
  ];
}

/** Half-cell colour keeps the surface light without moving surrounding text. */
function userEdge(width: number, pal: Palette, half: "▄" | "▀"): string {
  if (!hasColor()) return blank(width, pal.surface.subtle);
  return row(width, [{ text: half.repeat(width), fg: pal.surface.subtle }]);
}

export function renderAnswer(block: AnswerBlock, width: number, pal: Palette): string[] {
  return [
    "",
    ...markdown(block.text, width, pal, width).map((line) => row(width, line.segs)),
  ];
}

export function renderReasoning(
  block: ReasoningBlock,
  width: number,
  pal: Palette,
  context: { continues?: boolean } = {},
): string[] {
  // Expanding a live stream is deferred until it is sealed. Re-parsing an
  // ever-growing full thought on every token makes the whole TUI stall.
  const expanded = block.expanded === true && block.live !== true;
  const source = !expanded
    ? reasoningPreviewSource(block.text, width)
    : { text: block.text, truncated: false };
  const content = markdown(source.text, width, pal, width);
  const visible = expanded ? content : content.slice(-REASONING_PREVIEW_ROWS);

  return [
    ...(context.continues === true ? [] : [""]),
    ...visible.map((line) =>
      row(
        width,
        line.segs.map((seg) => ({ ...seg, fg: pal.ink.dim, italic: true })),
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
