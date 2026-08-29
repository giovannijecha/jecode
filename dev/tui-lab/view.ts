// The lab is a stateful catalogue around the production frame composer.

import type { Size } from "../../src/tui/screen.ts";
import { compose } from "../../src/tui/view.ts";
import { matches } from "../../src/tui/complete.ts";
import { SCENES } from "./model.ts";
import type { LabFrame, LabState, Scene } from "./model.ts";
import { sceneView } from "./scenes.ts";

export { SCENES } from "./model.ts";
export type { LabFrame, LabState, Scene } from "./model.ts";

export function composeLab(state: LabState, size: Size): LabFrame {
  const frame = compose(sceneView(state), size);
  return { rows: frame.rows, cursor: frame.cursor };
}

export function choiceCount(scene: Scene): number {
  if (scene === "diff") return 3;
  if (scene === "commands") return Math.min(4, matches("/").length);
  if (scene === "settings") return 10;
  return 1;
}
