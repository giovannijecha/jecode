// Shared grapheme boundaries for every projection of user-visible text.

const SEGMENTER = new Intl.Segmenter(undefined, { granularity: "grapheme" });

export function segmentGraphemes(text: string): Intl.Segments {
  return SEGMENTER.segment(text);
}

export function graphemes(text: string): string[] {
  const out: string[] = [];
  for (const { segment } of segmentGraphemes(text)) out.push(segment);
  return out;
}

/** Largest complete grapheme boundary no greater than a UTF-16 offset. */
export function graphemeFloor(text: string, offset: number): number {
  const target = Math.max(0, Math.min(text.length, offset));
  if (target === 0 || target === text.length) return target;
  const containing = segmentGraphemes(text).containing(target);
  if (containing === undefined || containing.index === target) return target;
  return containing.index;
}

/** Smallest complete grapheme boundary no less than a UTF-16 offset. */
export function graphemeCeiling(text: string, offset: number): number {
  const target = Math.max(0, Math.min(text.length, offset));
  if (target === 0 || target === text.length) return target;
  const containing = segmentGraphemes(text).containing(target);
  if (containing === undefined || containing.index === target) return target;
  return containing.index + containing.segment.length;
}

/** Keep a bounded prefix without returning part of a user-perceived character. */
export function leadingText(text: string, maxCodeUnits: number): string {
  if (maxCodeUnits <= 0) return "";
  if (text.length <= maxCodeUnits) return text;
  return text.slice(0, graphemeFloor(text, maxCodeUnits));
}

/** Keep a bounded suffix without returning part of a user-perceived character. */
export function trailingText(text: string, maxCodeUnits: number): string {
  if (maxCodeUnits <= 0) return "";
  if (text.length <= maxCodeUnits) return text;
  return text.slice(graphemeCeiling(text, text.length - maxCodeUnits));
}
