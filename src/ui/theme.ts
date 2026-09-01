// Jecode's fixed terminal identity. Components depend on semantic tokens, not
// literal colours, while the product exposes one deliberate dark Steel look.

export type RGB = readonly [number, number, number];

export type Ink = {
  readonly fg: RGB;
  readonly bright: RGB;
  readonly muted: RGB;
  readonly attention: RGB;
  readonly added: RGB;
  readonly removed: RGB;
};

export type Surface = {
  readonly subtle: RGB;
  readonly inset: RGB;
  readonly added: RGB;
  readonly removed: RGB;
  readonly attention: RGB;
};

export type Palette = {
  readonly accent: RGB;
  readonly accentSoft: RGB;
  readonly focus: RGB;
  readonly rule: RGB;
  readonly ink: Ink;
  readonly surface: Surface;
};

// Jecode's fixed dark Steel baseline. Structural blues, semantic outcomes,
// and slate surfaces keep the transcript vivid without turning it decorative.
// Components depend on these roles rather than embedding presentation values.
export const STEEL: Palette = {
  accent: [102, 155, 210],
  accentSoft: [131, 213, 245],
  focus: [102, 155, 210],
  rule: [53, 80, 110],
  ink: {
    fg: [212, 218, 225],
    bright: [235, 239, 244],
    muted: [112, 124, 137],
    attention: [230, 191, 95],
    added: [134, 203, 146],
    removed: [232, 112, 112],
  },
  surface: {
    subtle: [31, 38, 47],
    inset: [42, 52, 66],
    added: [22, 55, 34],
    removed: [62, 24, 27],
    attention: [62, 50, 19],
  },
};
