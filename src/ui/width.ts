// How wide a string actually is on screen.
//
// A row is laid out in cells, not in characters, and `.length` counts UTF-16
// code units — wrong three ways at once. An emoji is two units and two cells,
// a CJK glyph is one unit and two cells, a combining accent is one unit and no
// cell at all. Every alignment in the UI depends on this file being right:
// the right-hand column, a ground band, the cursor. Nothing measures with
// `.length`.

/** Ranges the terminal draws two cells wide (East Asian Wide and Fullwidth). */
const WIDE: readonly (readonly [number, number])[] = [
  [0x1100, 0x115f],
  [0x2e80, 0x303e],
  [0x3041, 0x33ff],
  [0x3400, 0x4dbf],
  [0x4e00, 0x9fff],
  [0xa000, 0xa4cf],
  [0xa960, 0xa97f],
  [0xac00, 0xd7a3],
  [0xf900, 0xfaff],
  [0xfe10, 0xfe19],
  [0xfe30, 0xfe6f],
  [0xff00, 0xff60],
  [0xffe0, 0xffe6],
  [0x1f300, 0x1f64f],
  [0x1f680, 0x1f6ff],
  [0x1f900, 0x1f9ff],
  [0x1fa70, 0x1faff],
  [0x20000, 0x3fffd],
];

/** Ranges that occupy no cell of their own: they attach to what precedes. */
const ZERO: readonly (readonly [number, number])[] = [
  [0x0300, 0x036f],
  [0x0483, 0x0489],
  [0x0591, 0x05bd],
  [0x0610, 0x061a],
  [0x064b, 0x065f],
  [0x0e31, 0x0e31],
  [0x0e34, 0x0e3a],
  [0x1ab0, 0x1aff],
  [0x1dc0, 0x1dff],
  [0x200b, 0x200f],
  [0x20d0, 0x20ff],
  [0xfe00, 0xfe0f],
  [0xfe20, 0xfe2f],
];

function inRanges(code: number, ranges: readonly (readonly [number, number])[]): boolean {
  for (const [lo, hi] of ranges) {
    if (code < lo) return false;
    if (code <= hi) return true;
  }
  return false;
}

// The emoji presentation selector, built rather than typed: an invisible byte
// in source is a byte nobody reviews.
const VS16 = String.fromCodePoint(0xfe0f);

// Grapheme segmentation is in the standard library, so a family emoji built
// out of five code points and three joiners counts as the one thing the
// terminal actually draws.
const SEGMENTER = new Intl.Segmenter(undefined, { granularity: "grapheme" });

export function graphemes(text: string): string[] {
  const out: string[] = [];
  for (const { segment } of SEGMENTER.segment(text)) out.push(segment);
  return out;
}

/**
 * Cells taken by one grapheme cluster.
 *
 * The base code point decides, with one exception: U+FE0F asks for the emoji
 * presentation of a character that would otherwise be drawn narrow, and every
 * terminal that honours it gives that glyph two cells.
 */
export function charWidth(cluster: string): number {
  const code = cluster.codePointAt(0);
  if (code === undefined) return 0;
  if (code < 0x20 || code === 0x7f) return 0;
  if (inRanges(code, ZERO)) return 0;
  if (cluster.includes(VS16)) return 2;
  return inRanges(code, WIDE) ? 2 : 1;
}

export function textWidth(text: string): number {
  let total = 0;
  for (const cluster of SEGMENTER.segment(text)) total += charWidth(cluster.segment);
  return total;
}

/** The longest prefix of `text` that fits in `cols` cells. */
export function clip(text: string, cols: number): string {
  if (cols <= 0) return "";
  let out = "";
  let used = 0;
  for (const { segment } of SEGMENTER.segment(text)) {
    const w = charWidth(segment);
    if (used + w > cols) break;
    out += segment;
    used += w;
  }
  return out;
}

/**
 * Fit `text` to `cols`, marking what was dropped.
 *
 * A row is recognized by where it begins and what it ends in, so a value too
 * long to show keeps both of its ends and loses the middle. An unmarked cut
 * reads as a whole shorter thing — a truncated path looks like a real path
 * that happens to be elsewhere.
 */
export function elide(text: string, cols: number): string {
  const width = textWidth(text);
  if (width <= cols) return text;
  if (cols <= 1) return clip("…", cols);

  const keep = cols - 1;
  const head = Math.ceil(keep / 2);
  const tail = keep - head;
  const clusters = graphemes(text);

  let front = "";
  let used = 0;
  for (const cluster of clusters) {
    const w = charWidth(cluster);
    if (used + w > head) break;
    front += cluster;
    used += w;
  }

  let back = "";
  used = 0;
  for (let i = clusters.length - 1; i >= 0; i--) {
    const cluster = clusters[i] as string;
    const w = charWidth(cluster);
    if (used + w > tail) break;
    back = cluster + back;
    used += w;
  }

  return `${front}…${back}`;
}

/**
 * Word wrap measured in cells, honouring existing newlines.
 *
 * A word wider than the row is broken rather than allowed to overflow: the
 * screen has autowrap off, so an overflowing row is not ugly, it is *gone* —
 * the terminal drops what does not fit and the user never learns it was there.
 */
export function wrapText(text: string, max: number, continuation = ""): string[] {
  if (max <= 0) return [text];
  // A continuation as wide as the row would leave no room for the text itself.
  const lead = textWidth(continuation) >= max ? 0 : textWidth(continuation);
  const out: string[] = [];

  for (const source of text.split("\n")) {
    let line = "";
    let width = 0;
    let first = true;

    const flush = (): void => {
      out.push(first || lead === 0 ? line : continuation + line);
      first = false;
      line = "";
      width = 0;
    };

    const room = (): number => max - (first ? 0 : lead);

    for (const word of source.split(" ")) {
      const w = textWidth(word);

      if (line !== "" && width + 1 + w > room()) flush();

      if (w > room()) {
        // Too long for any row: spend whole rows on it until it fits.
        let rest = word;
        while (textWidth(rest) > room()) {
          const head = clip(rest, room() - (line === "" ? 0 : width + 1));
          if (head === "") break;
          line = line === "" ? head : `${line} ${head}`;
          rest = rest.slice(head.length);
          flush();
        }
        line = rest;
        width = textWidth(rest);
        continue;
      }

      line = line === "" ? word : `${line} ${word}`;
      width = line === word ? w : width + 1 + w;
    }

    flush();
  }

  return out;
}
