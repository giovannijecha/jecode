import type { Palette } from "../../src/ui/theme.ts";
import type { Cursor } from "../../src/tui/frame.ts";

export const SCENES = [
  "golden",
  "conversation",
  "tools-live",
  "tools-trace",
  "tools-output",
  "tools-stream",
  "tools-diff",
  "approve-edit",
  "approve-command",
  "approve-denied",
  "menu-commands",
  "menu-search",
  "menu-settings",
  "help",
  "field",
  "markdown",
  "reasoning",
  "feedback",
] as const;
export type Scene = (typeof SCENES)[number];

export const ANIMATED: ReadonlySet<Scene> = new Set([
  "tools-live",
  "tools-trace",
  "tools-stream",
  "reasoning",
]);

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
