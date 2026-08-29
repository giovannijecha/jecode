// Inline Markdown notation: the marks inside a line, spent on style.
//
// Its own module because two things need it — a paragraph and a table cell —
// and neither should have to import the other to get it.

import type { Palette, RGB } from "./theme.ts";
import type { Seg } from "./render.ts";

const INLINE =
  /`([^`]+)`|\*\*([^*]+)\*\*|__([^_]+)__|\*([^*\s][^*]*)\*|\[([^\]]+)\]\(([^)]+)\)/g;

/**
 * Inline notation to styled segments.
 *
 * Emphasis becomes weight and colour rather than a slant, and a link keeps its
 * label and drops its target: a URL spelled out mid-sentence costs more rows
 * than it earns, and the label is what the sentence was written around.
 */
export function inline(text: string, base: RGB, pal: Palette, bold = false): Seg[] {
  const { ink } = pal;
  const segs: Seg[] = [];
  let last = 0;

  INLINE.lastIndex = 0;
  for (let match = INLINE.exec(text); match !== null; match = INLINE.exec(text)) {
    if (match.index > last) {
      segs.push({ text: text.slice(last, match.index), fg: base, bold: bold || undefined });
    }

    const [, mono, strong, strongAlt, emphasis, label] = match;
    if (mono !== undefined) segs.push({ text: mono, fg: pal.accentSoft });
    else if (strong !== undefined) segs.push({ text: strong, fg: ink.bright, bold: true });
    else if (strongAlt !== undefined) segs.push({ text: strongAlt, fg: ink.bright, bold: true });
    else if (emphasis !== undefined) segs.push({ text: emphasis, fg: ink.bright });
    else if (label !== undefined) segs.push({ text: label, fg: pal.accent });

    last = match.index + match[0].length;
  }

  if (last < text.length) {
    segs.push({ text: text.slice(last), fg: base, bold: bold || undefined });
  }
  return segs.length === 0 ? [{ text: "", fg: base }] : segs;
}
