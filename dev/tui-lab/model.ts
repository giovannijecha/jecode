import type { Palette } from "../../src/ui/theme.ts";
import type { Cursor } from "../../src/tui/frame.ts";

export const SCENES = [
  "conversation",
  "tools",
  "diff",
  "commands",
  "field",
  "markdown",
  "settings",
  "reasoning",
  "feedback",
] as const;
export type Scene = (typeof SCENES)[number];

export type LabState = {
  scene: Scene;
  palette: Palette;
  expanded: boolean;
  selected: number;
  tick: number;
};

export type LabFrame = {
  rows: string[];
  cursor?: Cursor;
};
