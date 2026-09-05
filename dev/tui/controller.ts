// Lab controls own fixture time; production input owns the preview surface.

import type { Key } from "../../src/tui/keys.ts";
import type { Size } from "../../src/tui/screen.ts";
import type { LabState } from "./model.ts";
import { TICK_MS } from "./model.ts";
import { SCENARIOS, scenarioFor, sceneView } from "./registry.ts";
import { createPreview } from "./preview.ts";

export function createLab(initial: LabState) {
  const initialTime = Math.round(initial.tick * TICK_MS);
  if (!Number.isSafeInteger(initialTime) || initialTime < 0) throw new Error("invalid lab time");
  let state = { ...initial, tick: initialTime / TICK_MS };
  let playing = true;
  let focus: "catalogue" | "preview" = "catalogue";
  let preview = makePreview();

  function makePreview() {
    return createPreview(sceneView(state), {
      command(text) {
        const command = text.trim().split(/\s+/)[0];
        const moment = command === undefined ? undefined : scenarioFor(state.scene).routes?.[command];
        if (moment !== undefined) { openScene(state.scene, moment); return; }
        const scene = SCENARIOS.find((item) => "command" in item && item.command === command);
        if (scene !== undefined) select(scene.id);
      },
      pick(index) {
        const next = scenarioFor(state.scene).select?.(index);
        if (next !== undefined) select(next);
      },
    });
  }

  function select(scene: string): void {
    openScene(scene, 0);
  }

  function openScene(scene: string, time: number): void {
    scenarioFor(scene);
    preview?.close();
    state = { ...state, scene, tick: time / TICK_MS, selected: 0, scroll: 0 };
    preview = makePreview();
  }

  function update(now: number): void {
    state = { ...state, tick: now / TICK_MS };
    preview.sync(sceneView(state));
  }

  return {
    get state(): Readonly<LabState> { return state; },
    get focus() { return focus; },
    get playing() { return playing; },
    get exited() { return preview.exited; },
    get animated() {
      const scene = scenarioFor(state.scene);
      return scene.animated === true && !preview.interrupted &&
        (scene.durationMs === undefined || state.tick * TICK_MS < scene.durationMs);
    },
    view() { return preview.view(); },
    render(size: Size) { return preview.render(size); },
    invalidate() { preview.invalidate(); },
    select,
    navigate(step: number) {
      const at = SCENARIOS.findIndex((scene) => scene.id === state.scene);
      const next = ((at + step) % SCENARIOS.length + SCENARIOS.length) % SCENARIOS.length;
      select(SCENARIOS[next]!.id);
    },
    restart() { select(state.scene); },
    nextMoment() {
      const moments = scenarioFor(state.scene).moments;
      const now = Math.round(state.tick * TICK_MS);
      const next = moments?.find((moment) => moment.time > now) ?? moments?.[0];
      if (next === undefined) return;
      openScene(state.scene, next.time);
      playing = false;
    },
    togglePlayback() { playing = !playing; },
    setPlaying(value: boolean) { playing = value; },
    toggleFocus() { focus = focus === "catalogue" ? "preview" : "catalogue"; },
    setFocus(value: "catalogue" | "preview") { focus = value; },
    setReducedMotion(value: boolean) {
      state = { ...state, reducedMotion: value };
      preview.style(state.palette, value);
    },
    advance(milliseconds = TICK_MS) {
      if (!Number.isSafeInteger(milliseconds) || milliseconds < 0) throw new Error("invalid lab time step");
      if (preview.interrupted || !scenarioFor(state.scene).animated) return;
      update(Math.round(state.tick * TICK_MS) + milliseconds);
    },
    handle(key: Key) { preview.handle(key); },
    close() { preview.close(); },
  };
}

export type Lab = ReturnType<typeof createLab>;
