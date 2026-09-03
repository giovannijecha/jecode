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
  const matcher = new RegExp(INLINE.source, INLINE.flags);

  for (let match = matcher.exec(text); match !== null; match = matcher.exec(text)) {
    if (match.index > last) {
      segs.push({ text: text.slice(last, match.index), fg: base, bold: bold || undefined });
    }

    const [, mono, strong, strongAlt, emphasis, label] = match;
    if (mono !== undefined) segs.push({ text: mono, fg: pal.technical, bold: bold || undefined });
    else if (strong !== undefined) segs.push(...inline(strong, ink.bright, pal, true));
    else if (strongAlt !== undefined) segs.push(...inline(strongAlt, ink.bright, pal, true));
    else if (emphasis !== undefined) segs.push(...inline(emphasis, ink.bright, pal, bold));
    else if (label !== undefined) segs.push({ text: label, fg: pal.technical, bold: bold || undefined });

    last = match.index + match[0].length;
  }

  if (last < text.length) {
    segs.push({ text: text.slice(last), fg: base, bold: bold || undefined });
  }
  return segs.length === 0 ? [{ text: "", fg: base }] : segs;
}
