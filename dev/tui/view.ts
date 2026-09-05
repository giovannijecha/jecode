// Static snapshots and interactive previews use the same controller/composer.

import type { Size } from "../../src/tui/screen.ts";
import type { LabFrame, LabState } from "./model.ts";
import { createLab } from "./controller.ts";

export { SCENES } from "./registry.ts";
export type { Scene } from "./registry.ts";
export type { LabFrame, LabState } from "./model.ts";

export function composeLab(state: LabState, size: Size): LabFrame {
  const lab = createLab(state);
  try {
    const frame = lab.render(size);
    return { rows: frame.rows, cursor: frame.cursor };
  } finally {
    lab.close();
  }
}
