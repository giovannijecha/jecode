// Jecode's fixed terminal identity. Components depend on semantic tokens, not
// literal colours, while the product exposes one deliberate dark Steel look.
// Jecode's fixed dark Steel baseline. Components depend on these semantic
// roles rather than embedding presentation values of their own.
export const STEEL = {
    accent: [138, 190, 183],
    accentSoft: [0, 215, 255],
    focus: [95, 135, 255],
    rule: [80, 80, 80],
    ink: {
        fg: [212, 212, 212],
        bright: [212, 212, 212],
        muted: [128, 128, 128],
        attention: [255, 255, 0],
        added: [181, 189, 104],
        removed: [204, 102, 102],
    },
    surface: {
        subtle: [52, 53, 65],
        inset: [40, 40, 50],
        added: [40, 50, 40],
        removed: [60, 40, 40],
        attention: [58, 58, 74],
    },
};
