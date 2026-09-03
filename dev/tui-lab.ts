// Interactive catalogue for reviewing Jecode's production terminal UI.
//
// It uses the real screen lifecycle, key decoder, cell renderer and incremental
// frame painter. It never creates a session, calls a provider or executes a
// tool: every state on screen is deterministic fixture data.

import { configureColor } from "../src/ui/render.ts";
import { STEEL } from "../src/ui/theme.ts";
import { painter } from "../src/tui/frame.ts";
import type { Key } from "../src/tui/keys.ts";
import { decoder } from "../src/tui/keys.ts";
import * as screen from "../src/tui/screen.ts";
import {
  choiceCount,
  labComposer,
  ANIMATED,
  SCENES,
} from "./tui-lab/view.ts";
import type { LabState, Scene } from "./tui-lab/view.ts";

const ESCAPE_MS = 25;

async function run(): Promise<void> {
  if (!screen.interactive()) {
    throw new Error("the TUI lab needs an interactive terminal");
  }

  let state: LabState = {
    scene: "golden",
    palette: STEEL,
    expanded: true,
    selected: 0,
    tick: 0,
  };

  configureColor(true);

  const paint = painter();
  const composeLab = labComposer();
  const keys = decoder();
  let live = true;
  let escapeTimer: NodeJS.Timeout | undefined;
  let close: (() => void) | undefined;
  const done = new Promise<void>((resolve) => {
    close = resolve;
  });

  const draw = (): void => {
    if (!live) return;
    const frame = composeLab(state, screen.size());
    paint.paint(frame.rows, frame.cursor);
  };

  const setState = (next: Partial<LabState>): void => {
    state = { ...state, ...next };
    draw();
  };

  const moveScene = (step: number): void => {
    const index = SCENES.indexOf(state.scene);
    setState({
      scene: cycle(SCENES, index, step),
      selected: 0,
    });
  };

  const moveChoice = (step: number): void => {
    const count = choiceCount(state.scene);
    setState({ selected: wrap(state.selected + step, count) });
  };

  const quit = (): void => {
    if (!live) return;
    live = false;
    close?.();
  };

  const handleChar = (char: string): void => {
    switch (char.toLocaleLowerCase()) {
      case "1":
        setState({ scene: "golden", selected: 0 });
        return;
      case "2":
        setState({ scene: "conversation", selected: 0 });
        return;
      case "3":
        setState({ scene: "tools-live", selected: 0 });
        return;
      case "4":
        setState({ scene: "tools-trace", selected: 0 });
        return;
      case "5":
        setState({ scene: "tools-output", selected: 0 });
        return;
      case "6":
        setState({ scene: "tools-stream", selected: 0 });
        return;
      case "7":
        setState({ scene: "tools-diff", selected: 0 });
        return;
      case "8":
        setState({ scene: "approve-edit", selected: 0 });
        return;
      case "9":
        setState({ scene: "approve-command", selected: 0 });
        return;
      case "o":
        setState({ expanded: !state.expanded });
        return;
      case "q":
        quit();
        return;
      default:
        return;
    }
  };

  const handle = (key: Key): void => {
    if (!live) return;

    if (key.ctrl && (key.name === "c" || key.name === "d")) {
      quit();
      return;
    }

    if (key.name === "escape") {
      if (state.scene === "approve-edit" || state.scene === "approve-command") {
        setState({ selected: choiceCount(state.scene) - 1 });
      }
      return;
    }

    if (key.name === "char" || key.name === "paste") {
      for (const char of key.text) handleChar(char);
      return;
    }

    switch (key.name) {
      case "left":
      case "pageup":
        moveScene(-1);
        return;
      case "right":
      case "pagedown":
        moveScene(1);
        return;
      case "up":
        moveChoice(-1);
        return;
      case "down":
      case "tab":
        moveChoice(1);
        return;
      case "home":
        setState({ scene: SCENES[0] as Scene, selected: 0 });
        return;
      case "end":
        setState({ scene: SCENES.at(-1) as Scene, selected: 0 });
        return;
      default:
        return;
    }
  };

  screen.enter(false);
  const animation = setInterval(() => {
    if (!live || !ANIMATED.has(state.scene)) return;
    state = { ...state, tick: state.tick + 1 };
    draw();
  }, 80);
  const stopResize = screen.onResize(() => {
    paint.invalidate();
    draw();
  });
  const stopInput = screen.onInput((chunk) => {
    for (const key of keys.push(chunk)) handle(key);
    if (escapeTimer !== undefined) clearTimeout(escapeTimer);
    escapeTimer = setTimeout(() => {
      escapeTimer = undefined;
      for (const key of keys.flush()) handle(key);
      draw();
    }, ESCAPE_MS);
    draw();
  });

  try {
    draw();
    await done;
  } finally {
    stopInput();
    stopResize();
    clearInterval(animation);
    if (escapeTimer !== undefined) clearTimeout(escapeTimer);
    screen.leave();
  }
}

function cycle<T>(values: readonly T[], index: number, step: number): T {
  return values[wrap(index + step, values.length)] as T;
}

function wrap(value: number, length: number): number {
  return ((value % length) + length) % length;
}

run().catch((error: unknown) => {
  screen.leave();
  process.stderr.write("jecode tui lab: " + (error as Error).message + "\n");
  process.exitCode = 1;
});
