import type { Command } from "../../commands.ts";
import type { Palette } from "../../ui/theme.ts";
import { renderMenu } from "./menu.ts";

const MAX_ROWS = 4;

export type CommandMenu = { rows: string[]; right: string };

export function renderCommandMenu(
  commands: readonly Command[],
  selected: number | undefined,
  width: number,
  pal: Palette,
): CommandMenu {
  const selectedIndex = selected === undefined ? 0 : Math.max(0, Math.min(commands.length - 1, selected));
  if (commands.length === 0) return { rows: [], right: "" };
  const menu = renderMenu(commands.map((command, index) => ({
    label: `/${command.name}`, selected: selectedIndex === index,
  })), width, pal, { maxRows: MAX_ROWS + 2, visible: MAX_ROWS });
  return {
    rows: menu.rows,
    right: commands.length > menu.last - menu.first ? `${menu.first + 1}–${menu.last} / ${commands.length}` : "",
  };
}

export function commandMenuLimit(): number {
  return MAX_ROWS;
}
