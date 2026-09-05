import type { Palette } from "../../src/ui/theme.ts";
import type { Cursor } from "../../src/tui/frame.ts";
import type { View } from "../../src/tui/view.ts";

export const TICK_MS = 80;

export type LabState = {
  scene: string;
  palette: Palette;
  expanded: boolean;
  selected: number;
  tick: number;
  reducedMotion?: boolean;
  scroll?: number;
};

export type Scenario = {
  id: string;
  title: string;
  group: "Conversation" | "Tools" | "Approvals" | "Input" | "Sessions" | "Configuration";
  create(state: LabState): View;
  animated?: boolean;
  durationMs?: number;
  command?: string;
  select?(index: number): string | undefined;
  moments?: readonly { title: string; time: number }[];
  /** Commands may open another inert sample within the same workflow. */
  routes?: Readonly<Record<string, number>>;
};

export type LabFrame = {
  rows: string[];
  cursor?: Cursor;
};
