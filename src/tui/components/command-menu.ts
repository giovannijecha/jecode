import type { Command } from "../../commands.ts";
import type { Palette } from "../../ui/theme.ts";
import { menuWindow, renderMenuRows } from "./menu.ts";

const MAX_ROWS = 4;

export type CommandMenu = { rows: string[]; right: string };

export function renderCommandMenu(
  commands: readonly Command[],
  selected: number | undefined,
  width: number,
  pal: Palette,
): CommandMenu {
  const selectedIndex = selected === undefined ? 0 : Math.max(0, Math.min(commands.length - 1, selected));
  const { first, last } = menuWindow(commands.length, selectedIndex, MAX_ROWS);
  const shown = commands.slice(first, last);
  if (shown.length === 0) return { rows: [], right: "" };
  return {
    rows: renderMenuRows(
      shown.map((command, index) => ({
        label: `/${command.name}`,
        description: command.blurb,
        selected: selectedIndex === first + index,
      })),
      width,
      pal,
    ),
    right: commands.length > shown.length ? `${first + 1}–${last} / ${commands.length}` : "",
  };
}

export function commandMenuLimit(): number {
  return MAX_ROWS;
}
