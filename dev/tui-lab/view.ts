// The lab is a stateful catalogue around the production frame composer.

import type { Size } from "../../src/tui/screen.ts";
import { compose } from "../../src/tui/view.ts";
import type { Block } from "../../src/tui/blocks.ts";
import { transcriptRenderer } from "../../src/tui/transcript-view.ts";
import { matches } from "../../src/tui/complete.ts";
import { SCENES } from "./model.ts";
import type { LabFrame, LabState, Scene } from "./model.ts";
import { sceneView } from "./scenes.ts";

export { ANIMATED, SCENES } from "./model.ts";
export type { LabFrame, LabState, Scene } from "./model.ts";

export function composeLab(state: LabState, size: Size): LabFrame {
  const frame = compose(sceneView(state), size);
  return { rows: frame.rows, cursor: frame.cursor };
}

/** Stateful production composer used by the interactive lab motion catalogue. */
export function labComposer(): (state: LabState, size: Size) => LabFrame {
  let now = 0;
  let previous: { scene: Scene; blocks: Block[] } | undefined;
  const transcript = transcriptRenderer(undefined, () => now);

  return (state, size) => {
    const view = sceneView(state);
    now = view.now ?? now;
    const blocks = previous?.scene === state.scene
      ? reconcile(previous.blocks, view.blocks)
      : [...view.blocks];

    if (previous?.scene !== state.scene) {
      transcript.invalidate();
      for (const block of blocks) {
        if (block.kind === "tool" && block.tone === "pending") transcript.invalidate(block);
      }
    } else {
      for (const block of blocks) transcript.invalidate(block);
    }
    previous = { scene: state.scene, blocks };

    const frame = compose({ ...view, blocks }, size, transcript);
    return { rows: frame.rows, cursor: frame.cursor };
  };
}

function reconcile(previous: readonly Block[], current: readonly Block[]): Block[] {
  return current.map((fresh, index) => {
    const retained = previous[index];
    if (retained === undefined || !sameBlock(retained, fresh)) return fresh;
    if (retained.kind === "tool") {
      delete retained.body;
      delete retained.durationMs;
      delete retained.expanded;
      delete retained.startedAt;
    }
    if (retained.kind === "reasoning") {
      delete retained.expanded;
      delete retained.live;
    }
    Object.assign(retained, fresh);
    return retained;
  });
}

function sameBlock(left: Block, right: Block): boolean {
  if (left.kind !== right.kind) return false;
  if (left.kind !== "tool" || right.kind !== "tool") return true;
  return left.name === right.name && left.target === right.target;
}

export function choiceCount(scene: Scene): number {
  if (scene === "approve-edit" || scene === "approve-command") return 3;
  if (scene === "menu-commands") return Math.min(4, matches("/").length);
  if (scene === "menu-settings") return 7;
  if (scene === "menu-permissions") return 7;
  if (scene === "menu-search") return 3;
  if (scene === "menu-resume") return 3;
  if (scene === "menu-timeline") return 4;
  return 1;
}
