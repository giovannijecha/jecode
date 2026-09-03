// Jecode's fixed terminal identity. Components depend on semantic tokens, not
// literal colours, while the product exposes one deliberate dark Slate look.

export type RGB = readonly [number, number, number];

export type Ink = {
  readonly fg: RGB;
  readonly bright: RGB;
  readonly muted: RGB;
  readonly dim: RGB;
  readonly attention: RGB;
  readonly added: RGB;
  readonly removed: RGB;
};

export type Surface = {
  readonly subtle: RGB;
  readonly added: RGB;
  readonly removed: RGB;
  readonly attention: RGB;
};

export type Palette = {
  readonly accent: RGB;
  readonly technical: RGB;
  readonly focus: RGB;
  readonly rule: RGB;
  readonly ink: Ink;
  readonly surface: Surface;
};

// Jecode's fixed dark Slate baseline. The exported name stays stable while
// cooler, quieter values leave the transcript bright enough to read and let
// live state carry the colour.
// Components depend on these roles rather than embedding presentation values.
export const STEEL: Palette = {
  accent: [124, 164, 222],
  technical: [126, 186, 208],
  focus: [124, 164, 222],
  rule: [44, 60, 78],
  ink: {
    fg: [214, 219, 226],
    bright: [241, 244, 248],
    muted: [149, 160, 174],
    dim: [99, 111, 125],
    attention: [226, 188, 112],
    added: [138, 190, 150],
    removed: [223, 120, 120],
  },
  surface: {
    subtle: [23, 29, 37],
    added: [21, 52, 33],
    removed: [58, 24, 27],
    attention: [58, 47, 20],
  },
};
