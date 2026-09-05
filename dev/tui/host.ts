// Terminal lifecycle and development controls around production previews.

import type { AppScreen } from "../../src/tui/app.ts";
import type { Painter } from "../../src/tui/frame.ts";
import { painter } from "../../src/tui/frame.ts";
import { decoder } from "../../src/tui/keys.ts";
import type { Key } from "../../src/tui/keys.ts";
import * as screen from "../../src/tui/screen.ts";
import type { Size } from "../../src/tui/screen.ts";
import { configureColor, hasColor, row } from "../../src/ui/render.ts";
import { STEEL } from "../../src/ui/theme.ts";
import { createLab } from "./controller.ts";
import type { Lab } from "./controller.ts";
import { TICK_MS } from "./model.ts";
import type { LabOptions } from "./options.ts";
import { SCENARIOS, scenarioFor } from "./registry.ts";

const CONTROL_ROWS = 3;
const SIZES: readonly (Size | undefined)[] = [undefined, { cols: 38, rows: 14 }, { cols: 100, rows: 30 }, { cols: 160, rows: 40 }];

export type LabEnvironment = { screen?: AppScreen; paint?: Painter; signal?: AbortSignal };

export async function runLab(options: LabOptions, environment: LabEnvironment = {}): Promise<void> {
  if (environment.screen === undefined && !screen.interactive()) {
    throw new Error("the TUI lab needs a terminal; use --render for a frame or --help for controls");
  }
  const terminal = environment.screen ?? screen;
  const paint = environment.paint ?? painter();
  const lab = createLab({
    scene: options.scene, palette: STEEL, expanded: true, selected: 0,
    tick: options.time / TICK_MS, reducedMotion: options.reducedMotion,
  });
  lab.setPlaying(!options.paused);
  let previewSize = options.size;
  let sizeIndex = 0;
  let color = options.color === "auto";
  configureColor(color);
  const keys = decoder({ ctrlBackspaceIsBs: process.env["WT_SESSION"] !== undefined });
  let timer: NodeJS.Timeout | undefined;
  let playbackTimer: NodeJS.Timeout | undefined;
  let escapeTimer: NodeJS.Timeout | undefined;
  let stopInput = () => {};
  let stopResize = () => {};
  let live = true;
  let failure: unknown;
  let finish = () => {};
  const done = new Promise<void>((resolve) => { finish = resolve; });
  const quit = (): void => { live = false; finish(); };
  const guard = (action: () => void): void => {
    if (!live) return;
    try { action(); } catch (error) { failure = error; quit(); }
  };
  const abort = (): void => { failure = environment.signal?.reason ?? new Error("interrupted"); quit(); };

  function draw(): void {
    if (!live) return;
    if (timer !== undefined) clearTimeout(timer);
    timer = undefined;
    const size = terminal.size();
    const height = Math.max(1, size.rows);
    const width = Math.max(1, size.cols);
    const controls = controlRows(lab, width).slice(0, height);
    const available = height - controls.length;
    let pending = false;
    let cursor;
    let rows = controls;
    if (available > 0) {
      const viewport = {
        cols: Math.min(width, previewSize?.cols ?? width),
        rows: Math.min(available, previewSize?.rows ?? available),
      };
      const frame = lab.render(viewport);
      pending = frame.transcriptPending;
      const padding = Math.max(0, available - frame.rows.length);
      rows = [...controls, ...Array.from({ length: padding }, () => ""), ...frame.rows];
      if (lab.focus === "preview" && frame.cursor !== undefined) {
        cursor = { ...frame.cursor, row: frame.cursor.row + controls.length + padding };
      }
    }
    paint.paint(rows, cursor);
    if (pending) timer = setTimeout(() => guard(draw), 16);
    if (lab.playing && lab.animated) {
      // Input and resize paints must not reset or accelerate fixture playback.
      playbackTimer ??= setTimeout(() => guard(() => {
        playbackTimer = undefined;
        lab.advance();
        draw();
      }), TICK_MS);
    } else if (playbackTimer !== undefined) {
      clearTimeout(playbackTimer);
      playbackTimer = undefined;
    }
  }

  function handle(key: Key): void {
    if (!live) return;
    if (key.ctrl && key.name === "g") { lab.toggleFocus(); return; }
    if (lab.focus === "preview") {
      lab.handle(key);
      if (lab.exited) quit();
      return;
    }
    if (key.ctrl && (key.name === "c" || key.name === "d")) { quit(); return; }
    switch (key.name) {
      case "left": case "up": case "pageup": lab.navigate(-1); return;
      case "right": case "down": case "pagedown": lab.navigate(1); return;
      case "home": lab.select(SCENARIOS[0].id); return;
      case "end": lab.select(SCENARIOS.at(-1)!.id); return;
      case "enter": case "tab": lab.setFocus("preview"); return;
      case "char": break;
      default: return;
    }
    switch (key.text.toLowerCase()) {
      case "q": quit(); break;
      case " ": case "p": lab.togglePlayback(); break;
      case ".": lab.advance(); break;
      case "r": lab.restart(); break;
      case "n": lab.nextMoment(); break;
      case "m":
        lab.setReducedMotion(!lab.state.reducedMotion);
        terminal.setReducedMotion(lab.state.reducedMotion === true);
        break;
      case "c":
        color = !color; configureColor(color);
        lab.invalidate(); paint.invalidate();
        break;
      case "[": case "]":
        sizeIndex = (sizeIndex + (key.text === "]" ? 1 : SIZES.length - 1)) % SIZES.length;
        previewSize = SIZES[sizeIndex];
        paint.invalidate();
        break;
    }
  }

  try {
    environment.signal?.throwIfAborted();
    terminal.enter(options.reducedMotion);
    stopResize = terminal.onResize(() => guard(() => { paint.invalidate(); draw(); }));
    stopInput = terminal.onInput((chunk) => guard(() => {
      for (const key of keys.push(chunk)) handle(key);
      if (escapeTimer !== undefined) clearTimeout(escapeTimer);
      escapeTimer = setTimeout(() => guard(() => {
        escapeTimer = undefined;
        for (const key of keys.flush()) handle(key);
        draw();
      }), 25);
      draw();
    }));
    environment.signal?.addEventListener("abort", abort, { once: true });
    draw();
    await done;
    if (failure !== undefined) throw failure;
  } finally {
    live = false;
    if (timer !== undefined) clearTimeout(timer);
    if (playbackTimer !== undefined) clearTimeout(playbackTimer);
    if (escapeTimer !== undefined) clearTimeout(escapeTimer);
    stopInput(); stopResize();
    environment.signal?.removeEventListener("abort", abort);
    lab.close();
    terminal.leave();
  }
}

function controlRows(lab: Lab, width: number): string[] {
  const scene = scenarioFor(lab.state.scene);
  const index = SCENARIOS.findIndex((item) => item.id === scene.id);
  const now = Math.round(lab.state.tick * TICK_MS);
  const playback = scene.durationMs !== undefined && now >= scene.durationMs ? "complete" : lab.playing ? "play" : "paused";
  const moment = scene.moments?.find((item) => item.time === now);
  const state = `${now}ms · ${playback} · motion ${lab.state.reducedMotion ? "reduced" : "full"} · color ${hasColor() ? "on" : "off"}${moment === undefined ? "" : ` · ${moment.title}`}`;
  const keys = lab.focus === "preview"
    ? "PREVIEW · production keys · ctrl+g catalogue"
    : `CATALOGUE · arrows scene · ${(scene.moments?.length ?? 0) > 0 ? "n sample · " : ""}enter preview · space pause · . step · r reset · m motion · c color · [ ] size · q quit`;
  return [
    row(width, [{ text: `${index + 1}/${SCENARIOS.length} ${scene.id} · ${scene.title}`, fg: STEEL.ink.attention }]),
    row(width, [{ text: state, fg: STEEL.ink.dim }]),
    row(width, [{ text: keys, fg: STEEL.ink.muted }]),
  ].slice(0, CONTROL_ROWS);
}
