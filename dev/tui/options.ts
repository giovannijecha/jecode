import type { Size } from "../../src/tui/screen.ts";
import { scenarioFor } from "./registry.ts";

export type LabOptions = {
  mode: "interactive" | "render" | "list" | "help";
  scene: string;
  size?: Size;
  color: "auto" | "off";
  reducedMotion: boolean;
  paused: boolean;
  time: number;
};

export const HELP = `TUI development lab

Usage: npm run tui:lab -- [options]

  --scene <id>           Open one catalogue scene
  --size <cols>x<rows>   Set preview dimensions (up to 300x200)
  --time <milliseconds> Start at an exact fixture time (0..3600000)
  --color <auto|off>    Use terminal colour or disable it; honors NO_COLOR
  --reduced-motion     Use resting tool states and a steady cursor
  --paused             Start with fixture playback paused
  --render             Write one preview frame without lab controls
  --list               List scene identifiers and titles
  --help               Show this reference

Catalogue: arrows choose a scene; Enter enters its preview.
Where available, n opens the next paused sample and resets preview edits.
Space pauses; . advances 80ms; r restarts; m toggles motion; c toggles colour.
[ and ] change preview size; q exits. Ctrl+G returns from preview to catalogue.
Preview keys use the production editor, pickers, scrolling, and steering.
All previews use inert data. No provider, tool, or persistent store is used.
`;

export function parseOptions(args: readonly string[]): LabOptions {
  const result: LabOptions = {
    mode: "interactive", scene: "golden", color: "auto",
    reducedMotion: false, paused: false, time: 0,
  };
  let modeSelected = false;
  for (let index = 0; index < args.length; index++) {
    const flag = args[index];
    const value = (): string => {
      const next = args[++index];
      if (next === undefined || next.startsWith("--")) throw new Error(`${flag} needs a value`);
      return next;
    };
    switch (flag) {
      case "--scene": result.scene = scenarioFor(value()).id; break;
      case "--size": {
        const size = /^(\d+)x(\d+)$/.exec(value());
        if (size === null) throw new Error("size must be cols x rows, for example 100x30");
        result.size = { cols: integer(size[1]!, 1, 300), rows: integer(size[2]!, 1, 200) };
        break;
      }
      case "--time": result.time = integer(value(), 0, 3_600_000); break;
      case "--color": {
        const color = value();
        if (color !== "auto" && color !== "off") throw new Error("color must be auto or off");
        result.color = color;
        break;
      }
      case "--reduced-motion": result.reducedMotion = true; break;
      case "--paused": result.paused = true; break;
      case "--render": case "--list": case "--help": {
        if (modeSelected) throw new Error("choose one of --render, --list, or --help");
        result.mode = flag.slice(2) as "render" | "list" | "help";
        modeSelected = true;
        break;
      }
      default: throw new Error(`unknown lab option: ${flag}`);
    }
  }
  return result;
}

function integer(value: string, min: number, max: number): number {
  const parsed = Number(value);
  if (!/^\d+$/.test(value) || !Number.isSafeInteger(parsed) || parsed < min || parsed > max) {
    throw new Error(`expected an integer from ${min} through ${max}`);
  }
  return parsed;
}
