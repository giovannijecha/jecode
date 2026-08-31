import type { Palette } from "../../ui/theme.ts";
import { blank, row } from "../../ui/render.ts";
import { markdown } from "../../ui/markdown.ts";
import type { AnswerBlock, ReasoningBlock, UserBlock } from "./types.ts";

const PAD = 1;
export const REASONING_PREVIEW_ROWS = 3;
const MIN_REASONING_PREVIEW_CHARS = 4_096;
const REASONING_PREVIEW_OVERSCAN = 12;

export function renderUser(block: UserBlock, width: number, pal: Palette): string[] {
  const inner = Math.max(8, width - PAD * 2);
  const content = markdown(block.text, inner, pal, inner);
  return [
    "",
    blank(width, pal.surface.subtle),
    ...content.map((line) => row(width, line.segs, [], pal.surface.subtle, PAD)),
    blank(width, pal.surface.subtle),
  ];
}

export function renderAnswer(block: AnswerBlock, width: number, pal: Palette): string[] {
  const inner = Math.max(8, width - PAD * 2);
  return [
    "",
    ...markdown(block.text, inner, pal, inner).map((line) =>
      row(width, line.segs, [], undefined, PAD)
    ),
  ];
}

export function renderReasoning(block: ReasoningBlock, width: number, pal: Palette): string[] {
  const inner = Math.max(8, width - PAD * 2);
  // Expanding a live stream is deferred until it is sealed. Re-parsing an
  // ever-growing full thought on every token makes the whole TUI stall.
  const expanded = block.expanded === true && block.live !== true;
  const source = !expanded
    ? reasoningPreviewSource(block.text, inner)
    : { text: block.text, truncated: false };
  const content = markdown(source.text, inner, pal, inner);
  const visible = expanded ? content : content.slice(-REASONING_PREVIEW_ROWS);
  const action = expanded
    ? "ctrl+o compact"
    : block.live === true && block.expanded === true
      ? "full when done"
    : source.truncated || content.length > REASONING_PREVIEW_ROWS
      ? "ctrl+o full"
      : undefined;

  return [
    "",
    row(
      width,
      [
        {
          text: block.live === true ? "thinking" : "thought",
          fg: pal.ink.bright,
          bold: true,
        },
      ],
      action === undefined ? [] : [{ text: action, fg: pal.ink.muted }],
      undefined,
      PAD,
    ),
    ...visible.map((line) =>
      row(
        width,
        line.segs.map((seg) => ({ ...seg, fg: pal.ink.muted, italic: true })),
        [],
        undefined,
        PAD,
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
  let start = text.length - limit;
  const code = text.charCodeAt(start);
  if (code >= 0xdc00 && code <= 0xdfff) start--;
  return { text: text.slice(start), truncated: true };
}
